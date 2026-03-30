//! LLVM backend for MAGI — translates MAGI IR to LLVM IR via inkwell.
//!
//! Pipeline: MAGI Source → AST → MAGI IR → LLVM IR → Machine Code
//! Runtime: C runtime library (magi_runtime.c) linked into every binary.

use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine, TargetTriple,
};
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::{BasicMetadataValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate, OptimizationLevel};

use super::ir::{tag, Instruction, IrFunction, IrModule};
use std::collections::HashMap;

/// Embedded C runtime source.
const RUNTIME_C_SOURCE: &str = include_str!("magi_runtime.c");

// NaN-boxing constants
const NANBOX_SIG: u64 = 0xFFF8_0000_0000_0000;
const PAYLOAD_MASK_U64: u64 = 0x0000_FFFF_FFFF_FFFF;
const NULL_TAG: u64 = NANBOX_SIG | ((tag::NULL as u64) << 48);
const BOOL_TAG: u64 = NANBOX_SIG | ((tag::BOOL as u64) << 48);
const I64_TAG: u64 = NANBOX_SIG | ((tag::I64 as u64) << 48);
const STRING_TAG: u64 = NANBOX_SIG | ((tag::STRING as u64) << 48);

/// Compile MAGI source to a native binary via LLVM.
pub fn compile_native(
    source: &str,
    source_path: Option<&str>,
    target_triple: Option<&str>,
    opt_level: u8,
    output_path: &str,
) -> Result<(), String> {
    let program = crate::syntax::parser::parse_v2(source)
        .map_err(|e| format!("parse error: {}", e.message))?;

    let mut compiler = super::Compiler::new();
    let ir_mod = compiler.compile(&program).map_err(|e| format!("{}", e))?;

    emit_native(&ir_mod, source_path, target_triple, opt_level, output_path)
}

/// Core LLVM pipeline: IR module → object file → linked binary.
/// All LLVM objects live in this function's scope to satisfy lifetime requirements.
fn emit_native(
    ir_mod: &IrModule,
    source_path: Option<&str>,
    target_triple: Option<&str>,
    opt_level: u8,
    output_path: &str,
) -> Result<(), String> {
    let ctx = Context::create();
    let module = ctx.create_module("magi_program");
    let b = ctx.create_builder();

    // Set the module's target triple to match the compilation target.
    // This ensures correct calling conventions (e.g., Windows x64 vs System V).
    // Normalize short target names to full LLVM triples.
    let norm_triple = match target_triple {
        Some(t) if t.contains("windows") && !t.contains("-pc-") => {
            let arch = t.split('-').next().unwrap_or("x86_64");
            Some(format!("{}-pc-windows-gnu", arch))
        }
        other => other.map(String::from),
    };
    let triple_ref = norm_triple.as_deref().or(target_triple);
    let mod_triple = triple_ref.map(TargetTriple::create).unwrap_or_else(TargetMachine::get_default_triple);
    module.set_triple(&mod_triple);

    let i64_t = ctx.i64_type();
    let i32_t = ctx.i32_type();
    let void_t = ctx.void_type();
    let ptr_t = ctx.ptr_type(AddressSpace::default());

    // ── Runtime function declarations ────────────────────────
    let mut rt: HashMap<&str, FunctionValue> = HashMap::new();
    macro_rules! rt_decl {
        ($name:expr, $ret:expr, [$($p:expr),*]) => {
            rt.insert($name, module.add_function($name, $ret.fn_type(&[$($p.into()),*], false), None));
        };
    }
    rt_decl!("__magi_print", void_t, [i64_t]);
    rt_decl!("__magi_runtime_call", i64_t, [ptr_t, i32_t, ptr_t]);
    rt_decl!("__magi_runtime_call_id", i64_t, [i32_t, i32_t, ptr_t]);
    rt_decl!("__magi_array_new", i64_t, [i32_t, ptr_t]);
    rt_decl!("__magi_array_get", i64_t, [i64_t, i64_t]);
    rt_decl!("__magi_array_set", void_t, [i64_t, i64_t, i64_t]);
    rt_decl!("__magi_array_len", i64_t, [i64_t]);
    rt_decl!("__magi_map_new", i64_t, [i32_t, ptr_t]);
    rt_decl!("__magi_map_get", i64_t, [i64_t, i64_t]);
    rt_decl!("__magi_map_set", void_t, [i64_t, i64_t, i64_t]);
    rt_decl!("__magi_string_concat", i64_t, [i64_t, i64_t]);
    rt_decl!("__magi_string_len", i64_t, [i64_t]);
    rt_decl!("__magi_to_string", i64_t, [i64_t]);

    // ── Direct builtin wrappers (bypass RuntimeCall dispatch) ──
    rt_decl!("__magi_builtin_len", i64_t, [i64_t]);
    rt_decl!("__magi_builtin_push", i64_t, [i64_t, i64_t]);
    rt_decl!("__magi_builtin_abs", i64_t, [i64_t]);
    rt_decl!("__magi_builtin_floor", i64_t, [i64_t]);
    rt_decl!("__magi_builtin_sqrt", i64_t, [i64_t]);
    rt_decl!("__magi_builtin_cos", i64_t, [i64_t]);
    rt_decl!("__magi_builtin_sin", i64_t, [i64_t]);
    rt_decl!("__magi_builtin_atan2", i64_t, [i64_t, i64_t]);

    // ── Globals ─────────────────────────────────────────────
    let mut globals: Vec<PointerValue> = Vec::new();
    for g in &ir_mod.globals {
        let gv = module.add_global(i64_t, None, &g.name);
        gv.set_initializer(&i64_t.const_int(NULL_TAG, false));
        globals.push(gv.as_pointer_value());
    }

    // ── Embedded data (from embed("file")) ────────────────
    let mut embedded_globals: Vec<(PointerValue, u64)> = Vec::new();
    for (i, ef) in ir_mod.embedded_data.iter().enumerate() {
        let bytes: Vec<inkwell::values::IntValue> = ef.data.iter()
            .map(|b| ctx.i8_type().const_int(*b as u64, false))
            .collect();
        let arr_val = ctx.i8_type().const_array(&bytes);
        let global_name = format!("__embed_data_{}", i);
        let gv = module.add_global(ctx.i8_type().array_type(ef.data.len() as u32), None, &global_name);
        gv.set_initializer(&arr_val);
        gv.set_constant(true);
        embedded_globals.push((gv.as_pointer_value(), ef.data.len() as u64));
    }

    // ── Declare functions ───────────────────────────────────
    // Use Vec indexed by IR function index to handle name collisions (lambdas)
    let mut fns_by_idx: Vec<FunctionValue> = Vec::new();
    let mut fns: HashMap<String, FunctionValue> = HashMap::new();
    for func in &ir_mod.functions {
        let params: Vec<BasicMetadataTypeEnum> = (0..func.param_count).map(|_| i64_t.into()).collect();
        let ft = i64_t.fn_type(&params, false);
        let lf = module.add_function(&func.name, ft, None);
        fns_by_idx.push(lf);
        fns.insert(func.name.clone(), lf);
    }

    // ── Compile function bodies ─────────────────────────────
    let mut str_cache: HashMap<u32, PointerValue> = HashMap::new();

    for (idx, func) in ir_mod.functions.iter().enumerate() {
        let lf = fns_by_idx[idx];
        compile_fn(&ctx, &module, &b, ir_mod, &fns_by_idx, &rt, &globals, &mut str_cache, &embedded_globals, func, lf)?;
    }

    // ── Function pointer table for indirect calls ─────────
    // Each MAGI function gets a wrapper: __magi_wrap_N(ptr args, i32 argc) -> i64
    // These wrappers are stored in a global table for __magi_call_fn.
    let wrap_fn_type = i64_t.fn_type(&[ptr_t.into(), i32_t.into()], false);
    let n_fns = ir_mod.functions.len();
    let table_type = ptr_t.array_type(n_fns as u32);
    let mut wrapper_ptrs: Vec<inkwell::values::PointerValue> = Vec::new();

    for (idx, func) in ir_mod.functions.iter().enumerate() {
        let wrap_name = format!("__magi_wrap_{}", idx);
        let wrapper = module.add_function(&wrap_name, wrap_fn_type, None);
        let entry = ctx.append_basic_block(wrapper, "entry");
        b.position_at_end(entry);

        let args_ptr = wrapper.get_nth_param(0).unwrap().into_pointer_value();
        let original = fns_by_idx[idx];
        let pc = func.param_count as usize;
        let mut call_args: Vec<BasicMetadataValueEnum> = Vec::new();
        for i in 0..pc {
            let gep = unsafe {
                b.build_gep(i64_t, args_ptr, &[i32_t.const_int(i as u64, false)], "ap").unwrap()
            };
            let val = b.build_load(i64_t, gep, "av").unwrap();
            call_args.push(val.into());
        }
        let result = b.build_call(original, &call_args, "wr").unwrap();
        if let Some(rv) = result.try_as_basic_value().left() {
            b.build_return(Some(&rv)).unwrap();
        } else {
            b.build_return(Some(&i64_t.const_int(NULL_TAG, false))).unwrap();
        }
        wrapper_ptrs.push(wrapper.as_global_value().as_pointer_value());
    }

    // Build global function table
    let fn_table = module.add_global(table_type, None, "__magi_fn_table");
    let table_init = ptr_t.const_array(&wrapper_ptrs);
    fn_table.set_initializer(&table_init);
    fn_table.set_linkage(inkwell::module::Linkage::External);

    // Build global function count
    let fn_count = module.add_global(i32_t, None, "__magi_fn_count");
    fn_count.set_initializer(&i32_t.const_int(n_fns as u64, false));
    fn_count.set_linkage(inkwell::module::Linkage::External);

    // ── main(argc, argv) entry point ──────────────────────────
    if let Some(mf) = fns.get("__main").copied() {
        let main_ft = i32_t.fn_type(&[i32_t.into(), ptr_t.into()], false);
        let main = module.add_function("main", main_ft, None);
        let entry = ctx.append_basic_block(main, "entry");
        b.position_at_end(entry);
        // Store argc/argv for process_args() runtime call
        let argc_global = module.add_global(i32_t, None, "__magi_argc");
        argc_global.set_linkage(inkwell::module::Linkage::External);
        let argv_global = module.add_global(ptr_t, None, "__magi_argv");
        argv_global.set_linkage(inkwell::module::Linkage::External);
        b.build_store(argc_global.as_pointer_value(), main.get_nth_param(0).unwrap()).unwrap();
        b.build_store(argv_global.as_pointer_value(), main.get_nth_param(1).unwrap()).unwrap();
        b.build_call(mf, &[], "r").unwrap();
        b.build_return(Some(&i32_t.const_int(0, false))).unwrap();
    }

    // ── Verify ──────────────────────────────────────────────
    if let Err(msg) = module.verify() {
        // Dump IR for debugging
        module.print_to_file("/tmp/magi_debug.ll").ok();
        return Err(format!("LLVM verification failed: {}", msg.to_string()));
    }

    // ── Target + optimize + emit ────────────────────────────
    let opt = match opt_level {
        0 => OptimizationLevel::None, 1 => OptimizationLevel::Less,
        2 => OptimizationLevel::Default, _ => OptimizationLevel::Aggressive,
    };
    Target::initialize_all(&InitializationConfig::default());
    let target = Target::from_triple(&mod_triple).map_err(|e| format!("invalid target: {}", e))?;
    let machine = target
        .create_target_machine(&mod_triple, "generic", "", opt, RelocMode::PIC, CodeModel::Default)
        .ok_or("failed to create target machine")?;

    let obj_path = format!("{}.o", output_path);
    machine.write_to_file(&module, FileType::Object, std::path::Path::new(&obj_path))
        .map_err(|e| format!("failed to write object: {}", e))?;

    let rt_path = format!("{}.magi_rt.c", output_path);
    std::fs::write(&rt_path, RUNTIME_C_SOURCE).map_err(|e| format!("write runtime: {}", e))?;

    let mut link_args: Vec<String> = vec![obj_path.clone(), rt_path.clone()];
    let mut temp_files: Vec<String> = vec![obj_path.clone(), rt_path.clone()];

    // Resolve native dependencies from packages with [native] in magi.toml
    let triple_str = target_triple.unwrap_or("");
    let is_windows = if triple_str.is_empty() { cfg!(target_os = "windows") } else { triple_str.contains("windows") };
    let is_macos = if triple_str.is_empty() { cfg!(target_os = "macos") } else { triple_str.contains("apple") || triple_str.contains("darwin") || triple_str.contains("macos") };
    let host_arch = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };
    let target_arch = if !triple_str.is_empty() {
        if triple_str.contains("aarch64") || triple_str.contains("arm64") { "aarch64" } else { "x86_64" }
    } else { host_arch };
    let platform_key = if is_windows { if target_arch == "aarch64" { "windows-aarch64" } else { "windows-x86_64" } }
        else if is_macos { if target_arch == "aarch64" { "macos-aarch64" } else { "macos-x86_64" } }
        else { if target_arch == "aarch64" { "linux-aarch64" } else { "linux-x86_64" } };
    let cc_cmd = if is_windows { "x86_64-w64-mingw32-gcc" } else { "cc" };

    for pkg_dir in find_native_packages(source_path) {
        let toml_path = format!("{}/magi.toml", pkg_dir);
        if let Ok(toml) = std::fs::read_to_string(&toml_path) {
            // Parse [native] section
            let mut in_native = false;
            for line in toml.lines() {
                let trimmed = line.trim();
                if trimmed == "[native]" { in_native = true; continue; }
                if trimmed.starts_with('[') && trimmed != "[native]" { in_native = false; continue; }
                if !in_native { continue; }

                if trimmed.starts_with("sources") {
                    for src in parse_toml_array(trimmed) {
                        let src_path = format!("{}/{}", pkg_dir, src);
                        if std::path::Path::new(&src_path).exists() {
                            link_args.push(src_path);
                        }
                    }
                }
                if trimmed.starts_with("include") {
                    for inc in parse_toml_array(trimmed) {
                        let inc_path = format!("{}/{}", pkg_dir, inc);
                        if std::path::Path::new(&inc_path).exists() {
                            link_args.push(format!("-I{}", inc_path));
                        }
                    }
                }
                if trimmed.starts_with("link-static") && trimmed.contains(platform_key) {
                    // Parse { linux-x86_64 = "path", windows-x86_64 = "path" }
                    if let Some(val) = extract_platform_value(trimmed, platform_key) {
                        let lib_path = format!("{}/{}", pkg_dir, val);
                        if std::path::Path::new(&lib_path).exists() {
                            link_args.push(lib_path);
                        }
                    }
                }
                if trimmed.starts_with("link-system-windows") && is_windows {
                    for lib in parse_toml_array(trimmed) {
                        link_args.push(format!("-l{}", lib));
                    }
                } else if trimmed.starts_with("link-system") && !trimmed.contains("windows") && !is_windows {
                    for lib in parse_toml_array(trimmed) {
                        link_args.push(format!("-l{}", lib));
                    }
                }
            }
        }
    }

    // Patch runtime to chain native package dispatchers
    
    if link_args.len() > 2 {
        // A native package was found — patch the runtime to call its dispatcher
        let patched_rt = format!("{}.patched.c", output_path);
        let rt_src = std::fs::read_to_string(&rt_path).unwrap_or_default();
        let patched = rt_src.replace(
            "// Unknown: return null",
            "{ extern int64_t __magi_runtime_call_hook(const char*, int32_t, int64_t*); int64_t __hook_r = __magi_runtime_call_hook(name, argc, args); if (__hook_r != magi_make_null()) return __hook_r; } // Unknown: return null"
        );
        std::fs::write(&patched_rt, patched).ok();
        // Replace rt_path in args
        link_args[1] = patched_rt.clone();
        temp_files.push(patched_rt);
    }

    link_args.push("-o".into());
    link_args.push(output_path.into());
    link_args.push("-lm".into());
    link_args.push("-O2".into());
    // 64MB stack for all platforms (RuntimeCall chains with map access are stack-heavy)
    if is_windows {
        link_args.push("-mconsole".into());
        link_args.push("-Wl,--stack,67108864".into());
    }
    if is_macos && std::env::consts::OS == "macos" {
        for fw in &["Cocoa", "IOKit", "CoreVideo", "CoreAudio", "AudioToolbox", "Carbon", "ForceFeedback"] {
            link_args.push("-framework".into());
            link_args.push(fw.to_string());
        }
        link_args.push("-liconv".into());
    }

    let status = std::process::Command::new(cc_cmd)
        .args(&link_args)
        .status()
        .map_err(|e| format!("linker: {}", e))?;

    if std::env::var("MAGI_KEEP_C").is_ok() {
        eprintln!("Keeping temp files: {:?}", temp_files);
    } else {
        for f in &temp_files { let _ = std::fs::remove_file(f); }
    }

    if !status.success() { return Err("linking failed".into()); }
    Ok(())
}

fn find_native_packages(source_path: Option<&str>) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let exe_dir = std::env::current_exe().ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let exe_packages = exe_dir.as_ref()
        .map(|d| d.join("../../packages").to_string_lossy().to_string());
    let mut search_paths_owned = vec![
        "packages".to_string(),
        "../packages".to_string(),
        "../../packages".to_string(),
    ];
    // Add search paths relative to the source file being compiled
    if let Some(sp) = source_path {
        let src_dir = std::path::Path::new(sp).parent();
        if let Some(dir) = src_dir {
            search_paths_owned.push(dir.join("packages").to_string_lossy().to_string());
            search_paths_owned.push(dir.join("../packages").to_string_lossy().to_string());
        }
    }
    let mut all_paths: Vec<&str> = search_paths_owned.iter().map(|s| s.as_str()).collect();
    if let Some(ref ep) = exe_packages { all_paths.push(ep.as_str()); }
    let search_paths = all_paths;
    for base in &search_paths {
        if let Ok(entries) = std::fs::read_dir(base) {
            for entry in entries.flatten() {
                let pkg_dir = entry.path().canonicalize().unwrap_or(entry.path());
                let toml = pkg_dir.join("magi.toml");
                if toml.exists() {
                    if let Ok(content) = std::fs::read_to_string(&toml) {
                        if content.contains("[native]") {
                            let key = pkg_dir.to_string_lossy().to_string();
                            if seen.insert(key.clone()) {
                                dirs.push(key);
                            }
                        }
                    }
                }
            }
        }
    }
    dirs
}

fn parse_toml_array(line: &str) -> Vec<String> {
    let mut result = Vec::new();
    if let Some(start) = line.find('[') {
        if let Some(end) = line.rfind(']') {
            let inner = &line[start+1..end];
            for item in inner.split(',') {
                let s = item.trim().trim_matches('"').trim_matches('\'').to_string();
                if !s.is_empty() { result.push(s); }
            }
        }
    }
    result
}

fn extract_platform_value(line: &str, platform: &str) -> Option<String> {
    if let Some(pos) = line.find(platform) {
        let after = &line[pos + platform.len()..];
        if let Some(eq) = after.find('=') {
            let val_part = &after[eq+1..];
            let trimmed = val_part.trim().trim_start_matches('"');
            if let Some(end) = trimmed.find('"') {
                return Some(trimmed[..end].to_string());
            }
        }
    }
    None
}

// ── Control flow stack entry ────────────────────────────────

struct CfEntry<'ctx> {
    branch_target: BasicBlock<'ctx>,
    merge_block: BasicBlock<'ctx>,
    else_block: Option<BasicBlock<'ctx>>,
    is_loop: bool,
    produces_value: bool,
    result_alloca: Option<PointerValue<'ctx>>,
    stack_depth: usize,
}

// ── Helper macros/functions ─────────────────────────────────

fn ci64<'a>(ctx: &'a Context, v: u64) -> IntValue<'a> { ctx.i64_type().const_int(v, false) }
fn cnull<'a>(ctx: &'a Context) -> IntValue<'a> { ci64(ctx, NULL_TAG) }

fn untag<'a>(b: &Builder<'a>, ctx: &'a Context, v: IntValue<'a>) -> IntValue<'a> {
    b.build_and(v, ci64(ctx, PAYLOAD_MASK_U64), "pay").unwrap()
}

fn sext48<'a>(b: &Builder<'a>, ctx: &'a Context, v: IntValue<'a>) -> IntValue<'a> {
    let s = b.build_left_shift(v, ci64(ctx, 16), "s16").unwrap();
    b.build_right_shift(s, ci64(ctx, 16), true, "sx").unwrap()
}

fn ext_i64<'a>(b: &Builder<'a>, ctx: &'a Context, v: IntValue<'a>) -> IntValue<'a> {
    sext48(b, ctx, untag(b, ctx, v))
}

fn tag_i64<'a>(b: &Builder<'a>, ctx: &'a Context, v: IntValue<'a>) -> IntValue<'a> {
    let m = b.build_and(v, ci64(ctx, PAYLOAD_MASK_U64), "m48").unwrap();
    b.build_or(m, ci64(ctx, I64_TAG), "ti").unwrap()
}

fn tag_bool<'a>(b: &Builder<'a>, ctx: &'a Context, c: IntValue<'a>) -> IntValue<'a> {
    let e = b.build_int_z_extend(c, ctx.i64_type(), "be").unwrap();
    b.build_or(e, ci64(ctx, BOOL_TAG), "tb").unwrap()
}

fn truthy<'a>(b: &Builder<'a>, ctx: &'a Context, v: IntValue<'a>) -> IntValue<'a> {
    let p = untag(b, ctx, v);
    b.build_int_compare(IntPredicate::NE, p, ci64(ctx, 0), "tr").unwrap()
}

fn to_f64<'a>(b: &Builder<'a>, ctx: &'a Context, v: IntValue<'a>) -> inkwell::values::FloatValue<'a> {
    b.build_bit_cast(v, ctx.f64_type(), "tf").unwrap().into_float_value()
}

fn from_f64<'a>(b: &Builder<'a>, ctx: &'a Context, v: inkwell::values::FloatValue<'a>) -> IntValue<'a> {
    b.build_bit_cast(v, ctx.i64_type(), "fi").unwrap().into_int_value()
}

fn str_ptr<'a>(
    b: &Builder<'a>, ir: &IrModule, cache: &mut HashMap<u32, PointerValue<'a>>, idx: u32,
) -> PointerValue<'a> {
    if let Some(&p) = cache.get(&idx) { return p; }
    let s = &ir.strings[idx as usize];
    let g = b.build_global_string_ptr(s, &format!("s{}", idx)).unwrap();
    let p = g.as_pointer_value();
    cache.insert(idx, p);
    p
}

fn icmp_push<'a>(b: &Builder<'a>, ctx: &'a Context, st: &mut Vec<IntValue<'a>>, pred: IntPredicate) {
    let bv = ext_i64(b, ctx, st.pop().unwrap_or(cnull(ctx)));
    let av = ext_i64(b, ctx, st.pop().unwrap_or(cnull(ctx)));
    let c = b.build_int_compare(pred, av, bv, "ic").unwrap();
    let r = b.build_int_z_extend(c, ctx.i64_type(), "ce").unwrap();
    st.push(tag_i64(b, ctx, r));
}

fn fcmp_push<'a>(b: &Builder<'a>, ctx: &'a Context, st: &mut Vec<IntValue<'a>>, pred: inkwell::FloatPredicate) {
    let bv = to_f64(b, ctx, st.pop().unwrap_or(cnull(ctx)));
    let av = to_f64(b, ctx, st.pop().unwrap_or(cnull(ctx)));
    let c = b.build_float_compare(pred, av, bv, "fc").unwrap();
    let r = b.build_int_z_extend(c, ctx.i64_type(), "fe").unwrap();
    st.push(tag_i64(b, ctx, r));
}

fn terminated(b: &Builder) -> bool {
    b.get_insert_block().map(|bb| bb.get_terminator().is_some()).unwrap_or(true)
}

// ── Numeric dispatch ID mapping ─────────────────────────────
// Maps RuntimeCall string names to enum MagiBuiltinId values.
// Must match the enum in magi_runtime.c exactly.
fn runtime_id(name: &str) -> Option<u32> {
    Some(match name {
        // Arithmetic
        "__add" => 1,
        "__sub" => 2,
        "__mul" => 3,
        "__div" => 4,
        "__mod" => 5,
        "__rem" => 6,
        "__eq" => 7,
        "__ne" => 8,
        "__lt" => 9,
        "__gt" => 10,
        "__le" => 11,
        "__ge" => 12,
        "__neg" => 13,
        "__pow" => 14,
        // Logical
        "__and" => 15,
        "__or" => 16,
        "__not" => 17,
        // Bitwise
        "__bit_and" => 18,
        "__bit_or" => 19,
        "__bit_xor" => 20,
        "__shl" | "__bit_shl" => 21,
        "__shr" | "__bit_shr" => 22,
        "__bit_not" => 23,
        "__bit_andnot" => 24,
        // Collections
        "len" => 25,
        "push" | "array_push" | "__array_push" => 26,
        "pop" | "array_pop" => 27,
        "has" => 28,
        "contains" => 29,
        "keys" => 30,
        "values" => 31,
        "entries" => 32,
        "map" => 33,
        "filter" => 34,
        "reduce" => 35,
        "find" => 36,
        "every" => 37,
        "some" => 38,
        "for_each" | "forEach" => 39,
        "reverse" => 40,
        "sort" => 41,
        "sort_by" | "sortBy" => 42,
        "flat_map" | "flatMap" => 43,
        "index_of" | "indexOf" => 44,
        "includes" => 45,
        // String
        "to_string" | "string" => 46,
        "typeof" | "type_of" => 47,
        "split" => 48,
        "join" => 49,
        "trim" => 50,
        "upper" | "to_upper" | "toUpperCase" => 51,
        "lower" | "to_lower" | "toLowerCase" => 52,
        "starts_with" | "startsWith" => 53,
        "ends_with" | "endsWith" => 54,
        "replace" => 55,
        "substring" | "substr" | "slice" => 56,
        "char_at" | "charAt" => 57,
        "concat" => 58,
        // Math
        "abs" => 59,
        "floor" => 60,
        "ceil" => 61,
        "sqrt" => 62,
        "round" => 63,
        "sin" => 64,
        "cos" => 65,
        "tan" => 66,
        "atan" => 67,
        "atan2" => 68,
        "asin" => 69,
        "acos" => 70,
        "pow" => 71,
        "fmod" => 72,
        "log" => 73,
        "log2" => 74,
        "log10" => 75,
        "exp" => 76,
        "min" => 77,
        "max" => 78,
        "random" => 79,
        "is_nan" | "isNaN" => 80,
        "is_finite" | "isFinite" => 81,
        // Range/slice
        "__range" => 82,
        "__slice" => 83,
        "__repeat" => 84,
        // Map
        "map_get" => 85,
        "map_set" => 86,
        "has_key" | "hasKey" => 87,
        // Parse
        "parse_int" => 88,
        "parse_float" => 89,
        "parse_json" | "json_parse" => 90,
        "stringify_json" | "json_stringify" | "to_json" => 91,
        // I/O
        "println" => 92,
        "print" => 93,
        // Process
        "exit" => 94,
        "panic" => 95,
        "timestamp_ms" | "time_ms" => 96,
        "process_args" | "args" => 97,
        "env_get" => 98,
        "env_set" => 99,
        "env_has" => 100,
        "exec_cmd" => 101,
        "cwd" => 102,
        "os_name" => 103,
        "pid" => 104,
        // Byte
        "__byte_slice" => 105,
        // File I/O
        "fs_read" | "file_read" | "read_file" => 106,
        "fs_write" | "file_write" | "write_file" => 107,
        "fs_exists" | "file_exists" => 108,
        "fs_delete" | "file_delete" | "delete_file" => 109,
        "fs_read_bytes" | "read_file_bytes" => 110,
        "fs_size" | "file_size" => 111,
        "fs_read_lines" => 112,
        "fs_mkdir" | "mkdir" | "create_dir" => 113,
        "file_append" | "append_file" => 114,
        "list_dir" | "read_dir" | "fs_list_dir" | "fs_list" => 115,
        // Path
        "path_join" => 116,
        // Renderers
        "__render_seg_cols" => 117,
        "__render_wall_col" => 118,
        "__render_flat_col" => 119,
        _ => return None,
    })
}

// ── Function body compilation ───────────────────────────────

#[allow(clippy::too_many_arguments)]
fn compile_fn<'ctx>(
    ctx: &'ctx Context,
    module: &Module<'ctx>,
    b: &Builder<'ctx>,
    ir: &IrModule,
    fns: &[FunctionValue<'ctx>],
    rt: &HashMap<&str, FunctionValue<'ctx>>,
    globals: &[PointerValue<'ctx>],
    str_cache: &mut HashMap<u32, PointerValue<'ctx>>,
    embedded: &[(PointerValue<'ctx>, u64)],
    func: &IrFunction,
    lf: FunctionValue<'ctx>,
) -> Result<(), String> {
    let entry = ctx.append_basic_block(lf, "entry");
    b.position_at_end(entry);
    let i64_t = ctx.i64_type();
    let i32_t = ctx.i32_type();

    // Locals
    let mut locals: Vec<PointerValue<'ctx>> = Vec::new();
    for (i, _) in func.locals.iter().enumerate() {
        let a = b.build_alloca(i64_t, &format!("l{}", i)).unwrap();
        if (i as u32) < func.param_count {
            b.build_store(a, lf.get_nth_param(i as u32).unwrap()).unwrap();
        } else {
            b.build_store(a, cnull(ctx)).unwrap();
        }
        locals.push(a);
    }

    // Pre-allocate a reusable args buffer for RuntimeCall (avoids alloca-in-loop stack overflow).
    let max_rc_args = func.instructions.iter().filter_map(|inst| {
        if let Instruction::RuntimeCall { arg_count, .. } = inst { Some(*arg_count as u64) }
        else if let Instruction::CallIndirect(arity) = inst { Some(*arity as u64) }
        else { None }
    }).max().unwrap_or(1).max(1);
    let rc_args_buf = b.build_array_alloca(i64_t, i32_t.const_int(max_rc_args, false), "rcbuf").unwrap();

    let mut stack: Vec<IntValue<'ctx>> = Vec::new();
    let mut cf: Vec<CfEntry<'ctx>> = Vec::new();
    let insts = &func.instructions;

    for ip in 0..insts.len() {
        // Skip dead code except structural control flow
        if terminated(b) {
            match &insts[ip] {
                Instruction::Block | Instruction::Loop | Instruction::If
                | Instruction::IfVoid | Instruction::Else | Instruction::End => {}
                _ => continue,
            }
        }

        match &insts[ip] {
            // ── Constants ──────────────────────────────
            Instruction::PushNull => stack.push(cnull(ctx)),
            Instruction::PushBool(v) => stack.push(ci64(ctx, BOOL_TAG | if *v { 1 } else { 0 })),
            Instruction::PushI64(n) => stack.push(ci64(ctx, tag::encode(tag::I64, *n) as u64)),
            Instruction::PushF64(f) => stack.push(ci64(ctx, f.to_bits())),
            Instruction::PushI32(n) => stack.push(ci64(ctx, tag::encode(tag::I64, *n as i64) as u64)),
            Instruction::PushF32(f) => stack.push(ci64(ctx, (*f as f64).to_bits())),
            Instruction::PushString(idx) => {
                let sp = str_ptr(b, ir, str_cache, *idx);
                let pi = b.build_ptr_to_int(sp, i64_t, "si").unwrap();
                stack.push(b.build_or(pi, ci64(ctx, STRING_TAG), "ts").unwrap());
            }

            // ── Locals/Globals ─────────────────────────
            Instruction::LocalGet(i) => {
                let v = b.build_load(i64_t, locals[*i as usize], "lg").unwrap();
                stack.push(v.into_int_value());
            }
            Instruction::LocalSet(i) => {
                let v = stack.pop().unwrap_or(cnull(ctx));
                b.build_store(locals[*i as usize], v).unwrap();
            }
            Instruction::LocalTee(i) => {
                let v = stack.last().copied().unwrap_or(cnull(ctx));
                b.build_store(locals[*i as usize], v).unwrap();
            }
            Instruction::GlobalGet(i) => {
                if let Some(&gp) = globals.get(*i as usize) {
                    let v = b.build_load(i64_t, gp, "gg").unwrap();
                    stack.push(v.into_int_value());
                } else { stack.push(cnull(ctx)); }
            }
            Instruction::GlobalSet(i) => {
                let v = stack.pop().unwrap_or(cnull(ctx));
                if let Some(&gp) = globals.get(*i as usize) { b.build_store(gp, v).unwrap(); }
            }

            // ── i64 arithmetic ─────────────────────────
            Instruction::I64Add => { let bv = ext_i64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); let av = ext_i64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); stack.push(tag_i64(b,ctx,b.build_int_add(av,bv,"a").unwrap())); }
            Instruction::I64Sub => { let bv = ext_i64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); let av = ext_i64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); stack.push(tag_i64(b,ctx,b.build_int_sub(av,bv,"s").unwrap())); }
            Instruction::I64Mul => { let bv = ext_i64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); let av = ext_i64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); stack.push(tag_i64(b,ctx,b.build_int_mul(av,bv,"m").unwrap())); }
            Instruction::I64Div => {
                let bv = ext_i64(b,ctx,stack.pop().unwrap_or(cnull(ctx)));
                let av = ext_i64(b,ctx,stack.pop().unwrap_or(cnull(ctx)));
                let is_zero = b.build_int_compare(IntPredicate::EQ, bv, i64_t.const_zero(), "dz").unwrap();
                let safe_b = b.build_select(is_zero, i64_t.const_int(1,false), bv, "sb").unwrap().into_int_value();
                let div = b.build_int_signed_div(av, safe_b, "d").unwrap();
                let res = b.build_select(is_zero, i64_t.const_zero(), div, "dr").unwrap().into_int_value();
                stack.push(tag_i64(b,ctx,res));
            }
            Instruction::I64Rem => {
                let bv = ext_i64(b,ctx,stack.pop().unwrap_or(cnull(ctx)));
                let av = ext_i64(b,ctx,stack.pop().unwrap_or(cnull(ctx)));
                let is_zero = b.build_int_compare(IntPredicate::EQ, bv, i64_t.const_zero(), "rz").unwrap();
                let safe_b = b.build_select(is_zero, i64_t.const_int(1,false), bv, "srb").unwrap().into_int_value();
                let rem = b.build_int_signed_rem(av, safe_b, "r").unwrap();
                let res = b.build_select(is_zero, i64_t.const_zero(), rem, "rr").unwrap().into_int_value();
                stack.push(tag_i64(b,ctx,res));
            }
            Instruction::I64Neg => {
                let val = stack.pop().unwrap_or(cnull(ctx));
                let masked = b.build_and(val, ci64(ctx, NANBOX_SIG), "nm").unwrap();
                let is_float = b.build_int_compare(IntPredicate::NE, masked, ci64(ctx, NANBOX_SIG), "isf").unwrap();
                let float_neg = b.build_xor(val, ci64(ctx, 0x8000_0000_0000_0000u64), "fn").unwrap();
                let int_neg = tag_i64(b, ctx, b.build_int_neg(ext_i64(b, ctx, val), "in").unwrap());
                let result = b.build_select(is_float, float_neg, int_neg, "neg").unwrap();
                stack.push(result.into_int_value());
            }

            // ── i64 comparisons ────────────────────────
            Instruction::I64Eq => icmp_push(b, ctx, &mut stack, IntPredicate::EQ),
            Instruction::I64Ne => icmp_push(b, ctx, &mut stack, IntPredicate::NE),
            Instruction::I64Lt => icmp_push(b, ctx, &mut stack, IntPredicate::SLT),
            Instruction::I64Gt => icmp_push(b, ctx, &mut stack, IntPredicate::SGT),
            Instruction::I64Le => icmp_push(b, ctx, &mut stack, IntPredicate::SLE),
            Instruction::I64Ge => icmp_push(b, ctx, &mut stack, IntPredicate::SGE),

            // ── f64 arithmetic ─────────────────────────
            Instruction::F64Add => { let bv=to_f64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); let av=to_f64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); stack.push(from_f64(b,ctx,b.build_float_add(av,bv,"fa").unwrap())); }
            Instruction::F64Sub => { let bv=to_f64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); let av=to_f64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); stack.push(from_f64(b,ctx,b.build_float_sub(av,bv,"fs").unwrap())); }
            Instruction::F64Mul => { let bv=to_f64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); let av=to_f64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); stack.push(from_f64(b,ctx,b.build_float_mul(av,bv,"fm").unwrap())); }
            Instruction::F64Div => { let bv=to_f64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); let av=to_f64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); stack.push(from_f64(b,ctx,b.build_float_div(av,bv,"fd").unwrap())); }
            Instruction::F64Neg => { let av=to_f64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); stack.push(from_f64(b,ctx,b.build_float_neg(av,"fn").unwrap())); }
            Instruction::F64Sqrt | Instruction::F64Floor | Instruction::F64Ceil | Instruction::F64Abs => {
                let av = to_f64(b, ctx, stack.pop().unwrap_or(cnull(ctx)));
                let intrinsic_name = match &insts[ip] {
                    Instruction::F64Sqrt => "llvm.sqrt.f64",
                    Instruction::F64Floor => "llvm.floor.f64",
                    Instruction::F64Ceil => "llvm.ceil.f64",
                    _ => "llvm.fabs.f64",
                };
                let ifn = module.get_function(intrinsic_name).unwrap_or_else(|| {
                    module.add_function(intrinsic_name, ctx.f64_type().fn_type(&[ctx.f64_type().into()], false), None)
                });
                let r = b.build_call(ifn, &[av.into()], "fi").unwrap();
                stack.push(from_f64(b, ctx, r.try_as_basic_value().left().unwrap().into_float_value()));
            }

            // ── f64 comparisons ────────────────────────
            Instruction::F64Eq => fcmp_push(b, ctx, &mut stack, inkwell::FloatPredicate::OEQ),
            Instruction::F64Ne => fcmp_push(b, ctx, &mut stack, inkwell::FloatPredicate::ONE),
            Instruction::F64Lt => fcmp_push(b, ctx, &mut stack, inkwell::FloatPredicate::OLT),
            Instruction::F64Gt => fcmp_push(b, ctx, &mut stack, inkwell::FloatPredicate::OGT),
            Instruction::F64Le => fcmp_push(b, ctx, &mut stack, inkwell::FloatPredicate::OLE),
            Instruction::F64Ge => fcmp_push(b, ctx, &mut stack, inkwell::FloatPredicate::OGE),

            // ── Logical / Conversions / Tags ───────────
            Instruction::BoolNot => { let v=stack.pop().unwrap_or(cnull(ctx)); let t=truthy(b,ctx,v); let n=b.build_not(t,"nt").unwrap(); stack.push(tag_bool(b,ctx,n)); }
            Instruction::I64ToF64 => { let v=ext_i64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); let f=b.build_signed_int_to_float(v,ctx.f64_type(),"i2f").unwrap(); stack.push(from_f64(b,ctx,f)); }
            Instruction::F64ToI64 => { let v=to_f64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); let i=b.build_float_to_signed_int(v,ctx.i64_type(),"f2i").unwrap(); stack.push(tag_i64(b,ctx,i)); }
            Instruction::TagI64 | Instruction::TagF64 | Instruction::TagBool | Instruction::TagString => {}
            Instruction::UntagI64 => { let v=stack.pop().unwrap_or(cnull(ctx)); stack.push(tag_i64(b,ctx,ext_i64(b,ctx,v))); }
            Instruction::UntagF64 | Instruction::UntagBool => { /* passthrough */ }
            Instruction::GetTag => {
                let v = stack.pop().unwrap_or(cnull(ctx));
                let sh = b.build_right_shift(v, ci64(ctx,48), false, "ts").unwrap();
                let tg = b.build_and(sh, ci64(ctx,7), "tv").unwrap();
                stack.push(tag_i64(b, ctx, tg));
            }

            // ── Control flow ───────────────────────────
            Instruction::Block => {
                let merge = ctx.append_basic_block(lf, "be");
                cf.push(CfEntry { branch_target: merge, merge_block: merge, else_block: None, is_loop: false, produces_value: false, result_alloca: None, stack_depth: stack.len() });
            }
            Instruction::Loop => {
                let header = ctx.append_basic_block(lf, "lh");
                let exit = ctx.append_basic_block(lf, "le");
                if !terminated(b) { b.build_unconditional_branch(header).unwrap(); }
                b.position_at_end(header);
                cf.push(CfEntry { branch_target: header, merge_block: exit, else_block: None, is_loop: true, produces_value: false, result_alloca: None, stack_depth: stack.len() });
            }
            Instruction::If => {
                let c = stack.pop().unwrap_or(cnull(ctx));
                let cb = truthy(b, ctx, c);
                let tb = ctx.append_basic_block(lf, "it");
                let eb = ctx.append_basic_block(lf, "ie");
                let mb = ctx.append_basic_block(lf, "im");
                let ra = b.build_alloca(i64_t, "ir").unwrap();
                b.build_store(ra, cnull(ctx)).unwrap();
                if !terminated(b) { b.build_conditional_branch(cb, tb, eb).unwrap(); }
                b.position_at_end(tb);
                cf.push(CfEntry { branch_target: mb, merge_block: mb, else_block: Some(eb), is_loop: false, produces_value: true, result_alloca: Some(ra), stack_depth: stack.len() });
            }
            Instruction::IfVoid => {
                let c = stack.pop().unwrap_or(cnull(ctx));
                let cb = truthy(b, ctx, c);
                let tb = ctx.append_basic_block(lf, "vt");
                let eb = ctx.append_basic_block(lf, "ve");
                let mb = ctx.append_basic_block(lf, "vm");
                if !terminated(b) { b.build_conditional_branch(cb, tb, eb).unwrap(); }
                b.position_at_end(tb);
                cf.push(CfEntry { branch_target: mb, merge_block: mb, else_block: Some(eb), is_loop: false, produces_value: false, result_alloca: None, stack_depth: stack.len() });
            }
            Instruction::Else => {
                if let Some(e) = cf.last() {
                    if e.produces_value {
                        if let Some(ra) = e.result_alloca {
                            let v = if stack.len() > e.stack_depth { stack.pop().unwrap_or(cnull(ctx)) } else { cnull(ctx) };
                            if !terminated(b) { b.build_store(ra, v).unwrap(); }
                        }
                    }
                    stack.truncate(e.stack_depth);
                    if !terminated(b) { b.build_unconditional_branch(e.merge_block).unwrap(); }
                    if let Some(eb) = e.else_block { b.position_at_end(eb); }
                }
            }
            Instruction::End => {
                if let Some(e) = cf.pop() {
                    if e.is_loop {
                        if !terminated(b) { b.build_unconditional_branch(e.branch_target).unwrap(); }
                        b.position_at_end(e.merge_block);
                        stack.truncate(e.stack_depth);
                    } else if e.produces_value {
                        if let Some(ra) = e.result_alloca {
                            let v = if stack.len() > e.stack_depth { stack.pop().unwrap_or(cnull(ctx)) } else { cnull(ctx) };
                            if !terminated(b) { b.build_store(ra, v).unwrap(); }
                        }
                        stack.truncate(e.stack_depth);
                        if !terminated(b) { b.build_unconditional_branch(e.merge_block).unwrap(); }
                        if let Some(eb) = e.else_block {
                            if eb.get_terminator().is_none() {
                                b.position_at_end(eb);
                                if let Some(ra) = e.result_alloca { b.build_store(ra, cnull(ctx)).unwrap(); }
                                b.build_unconditional_branch(e.merge_block).unwrap();
                            }
                        }
                        b.position_at_end(e.merge_block);
                        if let Some(ra) = e.result_alloca {
                            let r = b.build_load(i64_t, ra, "iv").unwrap();
                            stack.push(r.into_int_value());
                        }
                    } else {
                        stack.truncate(e.stack_depth);
                        if !terminated(b) { b.build_unconditional_branch(e.merge_block).unwrap(); }
                        if let Some(eb) = e.else_block {
                            if eb.get_terminator().is_none() {
                                b.position_at_end(eb);
                                b.build_unconditional_branch(e.merge_block).unwrap();
                            }
                        }
                        b.position_at_end(e.merge_block);
                    }
                }
            }
            Instruction::Br(depth) => {
                let idx = cf.len().saturating_sub(1 + *depth as usize);
                if let Some(e) = cf.get(idx) {
                    let t = if e.is_loop { e.branch_target } else { e.merge_block };
                    if !terminated(b) { b.build_unconditional_branch(t).unwrap(); }
                }
                let d = ctx.append_basic_block(lf, "pb");
                b.position_at_end(d);
            }
            Instruction::BrIf(depth) => {
                let c = stack.pop().unwrap_or(cnull(ctx));
                let cb = truthy(b, ctx, c);
                let idx = cf.len().saturating_sub(1 + *depth as usize);
                let cont = ctx.append_basic_block(lf, "bc");
                if let Some(e) = cf.get(idx) {
                    let t = if e.is_loop { e.branch_target } else { e.merge_block };
                    if !terminated(b) { b.build_conditional_branch(cb, t, cont).unwrap(); }
                }
                b.position_at_end(cont);
            }
            Instruction::BrTable(targets, default) => {
                let iv = ext_i64(b, ctx, stack.pop().unwrap_or(cnull(ctx)));
                let di = cf.len().saturating_sub(1 + *default as usize);
                let db = cf.get(di).map(|e| if e.is_loop { e.branch_target } else { e.merge_block })
                    .unwrap_or_else(|| ctx.append_basic_block(lf, "sd"));
                let cases: Vec<_> = targets.iter().enumerate().filter_map(|(i, d)| {
                    let ti = cf.len().saturating_sub(1 + *d as usize);
                    cf.get(ti).map(|e| (ctx.i64_type().const_int(i as u64, false), if e.is_loop { e.branch_target } else { e.merge_block }))
                }).collect();
                if !terminated(b) { b.build_switch(iv, db, &cases).unwrap(); }
                let d = ctx.append_basic_block(lf, "ps");
                b.position_at_end(d);
            }

            Instruction::Return => {
                let v = stack.pop().unwrap_or(cnull(ctx));
                b.build_return(Some(&v)).unwrap();
                let d = ctx.append_basic_block(lf, "pr");
                b.position_at_end(d);
            }
            Instruction::Unreachable => {
                b.build_unreachable().unwrap();
                let d = ctx.append_basic_block(lf, "pu");
                b.position_at_end(d);
            }
            Instruction::Nop => {}
            Instruction::Drop => { stack.pop(); }

            // ── Function calls ─────────────────────────
            Instruction::Call(fi) => {
                let tf = &ir.functions[*fi as usize];
                // Use index-based lookup to handle name collisions (lambdas)
                let tlf = fns[*fi as usize];
                let ac = tf.param_count as usize;
                let mut args: Vec<BasicMetadataValueEnum> = Vec::new();
                for _ in 0..ac { args.push(stack.pop().unwrap_or(cnull(ctx)).into()); }
                args.reverse();
                let r = b.build_call(tlf, &args, "c").unwrap();
                if let Some(v) = r.try_as_basic_value().left() { stack.push(v.into_int_value()); }
            }
            Instruction::CallIndirect(type_idx) => {
                // Indirect call: function index is on the stack, then args
                // type_idx is the arity (number of arguments)
                let fn_idx_val = stack.pop().unwrap_or(cnull(ctx));
                let arity = *type_idx as usize;
                let mut call_args: Vec<IntValue> = (0..arity).map(|_| stack.pop().unwrap_or(cnull(ctx))).collect();
                call_args.reverse();

                // Reuse pre-allocated args buffer
                for (i, a) in call_args.iter().enumerate() {
                    let p = unsafe { b.build_gep(i64_t, rc_args_buf, &[i32_t.const_int(i as u64, false)], "iap").unwrap() };
                    b.build_store(p, *a).unwrap();
                }

                // Declare __magi_call_fn if needed
                let call_fn = module.get_function("__magi_call_fn").unwrap_or_else(|| {
                    let ft = i64_t.fn_type(&[i64_t.into(), i32_t.into(), ctx.ptr_type(AddressSpace::default()).into()], false);
                    module.add_function("__magi_call_fn", ft, None)
                });
                let result = b.build_call(call_fn, &[fn_idx_val.into(), i32_t.const_int(arity as u64, false).into(), rc_args_buf.into()], "ic").unwrap();
                stack.push(result.try_as_basic_value().left().unwrap().into_int_value());
            }

            // ── Memory ops ─────────────────────────────
            Instruction::HeapAlloc(_) => {
                let a = b.build_alloca(i64_t, "h").unwrap();
                stack.push(b.build_ptr_to_int(a, i64_t, "hp").unwrap());
            }
            Instruction::MemLoadI64 | Instruction::MemLoadF64 => {
                let addr = stack.pop().unwrap_or(cnull(ctx));
                let p = b.build_int_to_ptr(addr, ctx.ptr_type(AddressSpace::default()), "mp").unwrap();
                let v = b.build_load(i64_t, p, "ml").unwrap();
                stack.push(v.into_int_value());
            }
            Instruction::MemStoreI64 | Instruction::MemStoreF64 => {
                let v = stack.pop().unwrap_or(cnull(ctx));
                let addr = stack.pop().unwrap_or(cnull(ctx));
                let p = b.build_int_to_ptr(addr, ctx.ptr_type(AddressSpace::default()), "mp").unwrap();
                b.build_store(p, v).unwrap();
            }
            Instruction::MemLoadI32 => {
                let addr = stack.pop().unwrap_or(cnull(ctx));
                let p = b.build_int_to_ptr(addr, ctx.ptr_type(AddressSpace::default()), "mp").unwrap();
                let v = b.build_load(i32_t, p, "ml32").unwrap();
                stack.push(b.build_int_s_extend(v.into_int_value(), i64_t, "sx32").unwrap());
            }
            Instruction::MemStoreI32 => {
                let v = stack.pop().unwrap_or(cnull(ctx));
                let addr = stack.pop().unwrap_or(cnull(ctx));
                let p = b.build_int_to_ptr(addr, ctx.ptr_type(AddressSpace::default()), "mp").unwrap();
                b.build_store(p, b.build_int_truncate(v, i32_t, "tr32").unwrap()).unwrap();
            }

            // ── Array/Map/String runtime ───────────────
            Instruction::ArrayNew(count) => {
                let n = *count;
                let aa = b.build_array_alloca(i64_t, i32_t.const_int(std::cmp::max(n,1) as u64, false), "at").unwrap();
                let mut elems: Vec<IntValue> = (0..n).map(|_| stack.pop().unwrap_or(cnull(ctx))).collect();
                elems.reverse();
                for (i, e) in elems.iter().enumerate() {
                    let p = unsafe { b.build_gep(i64_t, aa, &[i32_t.const_int(i as u64, false)], "ep").unwrap() };
                    b.build_store(p, *e).unwrap();
                }
                let r = b.build_call(rt["__magi_array_new"], &[i32_t.const_int(n as u64, false).into(), aa.into()], "an").unwrap();
                stack.push(r.try_as_basic_value().left().unwrap().into_int_value());
            }
            Instruction::ArrayGet => {
                let idx_val = stack.pop().unwrap_or(cnull(ctx));
                let arr_val = stack.pop().unwrap_or(cnull(ctx));

                // Check if arr_val is a NaN-boxed TAG_ARRAY: (v >> 48) & 7 == 4
                let arr_tag = b.build_and(
                    b.build_right_shift(arr_val, ci64(ctx, 48), false, "as48").unwrap(),
                    ci64(ctx, 7), "atg",
                ).unwrap();
                let is_array = b.build_int_compare(
                    IntPredicate::EQ, arr_tag, ci64(ctx, tag::ARRAY as u64), "ia",
                ).unwrap();

                let fast_bb = ctx.append_basic_block(lf, "aget_fast");
                let slow_bb = ctx.append_basic_block(lf, "aget_slow");
                let merge_bb = ctx.append_basic_block(lf, "aget_merge");

                b.build_conditional_branch(is_array, fast_bb, slow_bb).unwrap();

                // ── Fast path: inline array access ──
                b.position_at_end(fast_bb);

                // Extract MagiArray* from payload bits
                let ptr_raw = b.build_and(arr_val, ci64(ctx, PAYLOAD_MASK_U64), "pr").unwrap();
                let arr_ptr = b.build_int_to_ptr(
                    ptr_raw, ctx.ptr_type(AddressSpace::default()), "ap",
                ).unwrap();

                // Load cap at byte offset 12 (struct: {i64* data, i32 len, i32 cap})
                let cap_byte_ptr = unsafe {
                    b.build_gep(ctx.i8_type(), arr_ptr,
                        &[i32_t.const_int(12, false)], "cbp").unwrap()
                };
                let cap_val = b.build_load(i32_t, cap_byte_ptr, "cap")
                    .unwrap().into_int_value();

                // Check cap != -1 (not a byte array)
                let is_regular = b.build_int_compare(
                    IntPredicate::NE, cap_val,
                    i32_t.const_int(-1i32 as u32 as u64, false), "ir",
                ).unwrap();

                // Extract integer index from NaN-boxed value
                let idx_int = ext_i64(b, ctx, idx_val);
                let idx_i32 = b.build_int_truncate(idx_int, i32_t, "ii32").unwrap();

                // Load len at byte offset 8
                let len_byte_ptr = unsafe {
                    b.build_gep(ctx.i8_type(), arr_ptr,
                        &[i32_t.const_int(8, false)], "lbp").unwrap()
                };
                let len_val = b.build_load(i32_t, len_byte_ptr, "len")
                    .unwrap().into_int_value();

                // Bounds check: 0 <= idx < len
                let ge_zero = b.build_int_compare(
                    IntPredicate::SGE, idx_i32, i32_t.const_zero(), "gz",
                ).unwrap();
                let lt_len = b.build_int_compare(
                    IntPredicate::SLT, idx_i32, len_val, "ll",
                ).unwrap();
                let in_bounds = b.build_and(ge_zero, lt_len, "ib").unwrap();
                let can_inline = b.build_and(is_regular, in_bounds, "ci").unwrap();

                let inline_bb = ctx.append_basic_block(lf, "aget_inline");
                let oob_bb = ctx.append_basic_block(lf, "aget_oob");
                b.build_conditional_branch(can_inline, inline_bb, oob_bb).unwrap();

                // ── Inline load: arr->data[idx] ──
                b.position_at_end(inline_bb);
                // Load data pointer at byte offset 0
                let data_ptr = b.build_load(
                    ctx.ptr_type(AddressSpace::default()), arr_ptr, "dp",
                ).unwrap().into_pointer_value();
                let idx_u64 = b.build_int_z_extend(idx_i32, i64_t, "iu64").unwrap();
                let elem_ptr = unsafe {
                    b.build_gep(i64_t, data_ptr, &[idx_u64], "ep").unwrap()
                };
                let fast_result = b.build_load(i64_t, elem_ptr, "fr")
                    .unwrap().into_int_value();
                b.build_unconditional_branch(merge_bb).unwrap();

                // ── OOB / byte array: return null ──
                b.position_at_end(oob_bb);
                let oob_result = cnull(ctx);
                b.build_unconditional_branch(merge_bb).unwrap();

                // ── Slow path: maps, byte arrays → call C runtime ──
                b.position_at_end(slow_bb);
                let slow_r = b.build_call(
                    rt["__magi_array_get"],
                    &[arr_val.into(), idx_val.into()], "sg",
                ).unwrap();
                let slow_result = slow_r.try_as_basic_value().left()
                    .unwrap().into_int_value();
                b.build_unconditional_branch(merge_bb).unwrap();

                // ── Merge ──
                b.position_at_end(merge_bb);
                let phi = b.build_phi(i64_t, "aget_r").unwrap();
                phi.add_incoming(&[
                    (&fast_result, inline_bb),
                    (&oob_result, oob_bb),
                    (&slow_result, slow_bb),
                ]);
                stack.push(phi.as_basic_value().into_int_value());
            }
            Instruction::ArraySet => {
                let v = stack.pop().unwrap_or(cnull(ctx));
                let idx = stack.pop().unwrap_or(cnull(ctx));
                let arr = stack.pop().unwrap_or(cnull(ctx));
                b.build_call(rt["__magi_array_set"], &[arr.into(), idx.into(), v.into()], "").unwrap();
            }
            Instruction::ArrayLen => {
                let arr = stack.pop().unwrap_or(cnull(ctx));
                let r = b.build_call(rt["__magi_array_len"], &[arr.into()], "al").unwrap();
                stack.push(r.try_as_basic_value().left().unwrap().into_int_value());
            }
            Instruction::MapNew(count) => {
                let n = *count;
                let ea = b.build_array_alloca(i64_t, i32_t.const_int(std::cmp::max(n*2,1) as u64, false), "mt").unwrap();
                let mut pairs: Vec<(IntValue, IntValue)> = Vec::new();
                for _ in 0..n {
                    let v = stack.pop().unwrap_or(cnull(ctx));
                    let k = stack.pop().unwrap_or(cnull(ctx));
                    pairs.push((k, v));
                }
                pairs.reverse();
                for (i, (k, v)) in pairs.iter().enumerate() {
                    let kp = unsafe { b.build_gep(i64_t, ea, &[i32_t.const_int((i*2) as u64, false)], "kp").unwrap() };
                    b.build_store(kp, *k).unwrap();
                    let vp = unsafe { b.build_gep(i64_t, ea, &[i32_t.const_int((i*2+1) as u64, false)], "vp").unwrap() };
                    b.build_store(vp, *v).unwrap();
                }
                let r = b.build_call(rt["__magi_map_new"], &[i32_t.const_int(n as u64, false).into(), ea.into()], "mn").unwrap();
                stack.push(r.try_as_basic_value().left().unwrap().into_int_value());
            }
            Instruction::MapGet => {
                let k = stack.pop().unwrap_or(cnull(ctx));
                let m = stack.pop().unwrap_or(cnull(ctx));
                let r = b.build_call(rt["__magi_map_get"], &[m.into(), k.into()], "mg").unwrap();
                stack.push(r.try_as_basic_value().left().unwrap().into_int_value());
            }
            Instruction::MapSet => {
                let v = stack.pop().unwrap_or(cnull(ctx));
                let k = stack.pop().unwrap_or(cnull(ctx));
                let m = stack.pop().unwrap_or(cnull(ctx));
                b.build_call(rt["__magi_map_set"], &[m.into(), k.into(), v.into()], "").unwrap();
            }
            Instruction::StringConcat => {
                let bv = stack.pop().unwrap_or(cnull(ctx));
                let av = stack.pop().unwrap_or(cnull(ctx));
                let r = b.build_call(rt["__magi_string_concat"], &[av.into(), bv.into()], "sc").unwrap();
                stack.push(r.try_as_basic_value().left().unwrap().into_int_value());
            }
            Instruction::StringLen => {
                let s = stack.pop().unwrap_or(cnull(ctx));
                let r = b.build_call(rt["__magi_string_len"], &[s.into()], "sl").unwrap();
                stack.push(r.try_as_basic_value().left().unwrap().into_int_value());
            }

            // ── Print ──────────────────────────────────
            Instruction::Print => {
                let v = stack.pop().unwrap_or(cnull(ctx));
                b.build_call(rt["__magi_print"], &[v.into()], "").unwrap();
            }

            // ── RuntimeCall ────────────────────────────
            Instruction::RuntimeCall { name, arg_count } => {
                let fn_name = &ir.strings[*name as usize];

                // embed() — compile-time file embedding
                if fn_name == "__embed" {
                    let idx_val = stack.pop().unwrap_or(cnull(ctx));
                    let idx_raw = ext_i64(b, ctx, idx_val);
                    // For constant index (compile-time known), emit direct array creation
                    // The index was pushed as PushI64(idx) right before this call
                    if let Some(const_val) = idx_raw.get_zero_extended_constant() {
                        let ei = const_val as usize;
                        if ei < embedded.len() {
                            let (data_ptr, data_len) = embedded[ei];
                            // Call __magi_embed_array(ptr, len) → creates a MAGI array from raw bytes
                            let embed_fn = module.get_function("__magi_embed_array").unwrap_or_else(|| {
                                let ft = i64_t.fn_type(&[ctx.ptr_type(AddressSpace::default()).into(), i64_t.into()], false);
                                module.add_function("__magi_embed_array", ft, None)
                            });
                            let r = b.build_call(embed_fn, &[data_ptr.into(), ci64(ctx, data_len).into()], "emb").unwrap();
                            stack.push(r.try_as_basic_value().left().unwrap().into_int_value());
                        } else {
                            stack.push(cnull(ctx));
                        }
                    } else {
                        stack.push(cnull(ctx));
                    }
                } else {
                    // Direct call optimization for known builtins
                    let direct_builtin: Option<(&str, usize)> = match (fn_name.as_str(), *arg_count) {
                        ("len", 1) => Some(("__magi_builtin_len", 1)),
                        ("push" | "array_push" | "__array_push", 2) => Some(("__magi_builtin_push", 2)),
                        ("abs", 1) => Some(("__magi_builtin_abs", 1)),
                        ("floor", 1) => Some(("__magi_builtin_floor", 1)),
                        ("sqrt", 1) => Some(("__magi_builtin_sqrt", 1)),
                        ("cos", 1) => Some(("__magi_builtin_cos", 1)),
                        ("sin", 1) => Some(("__magi_builtin_sin", 1)),
                        ("atan2", 2) => Some(("__magi_builtin_atan2", 2)),
                        _ => None,
                    };

                    if let Some((c_name, expected_argc)) = direct_builtin {
                        let mut args: Vec<IntValue> = (0..(*arg_count)).map(|_| stack.pop().unwrap_or(cnull(ctx))).collect();
                        args.reverse();
                        args.truncate(expected_argc);
                        let call_args: Vec<BasicMetadataValueEnum> = args.iter().map(|a| (*a).into()).collect();
                        let r = b.build_call(rt[c_name], &call_args, "db").unwrap();
                        stack.push(r.try_as_basic_value().left().unwrap().into_int_value());
                    } else if *arg_count == 2 && matches!(fn_name.as_str(),
                        "__add" | "__sub" | "__mul" | "__div" | "__mod" | "__rem" |
                        "__lt" | "__gt" | "__le" | "__ge" | "__neg")
                    {
                        // ── Inline binary op: int fast path → float path → C fallback ──
                        let bv = stack.pop().unwrap_or(cnull(ctx));
                        let av = stack.pop().unwrap_or(cnull(ctx));

                        // Check if value is tagged: (v & NANBOX_SIG) == NANBOX_SIG
                        let a_masked = b.build_and(av, ci64(ctx, NANBOX_SIG), "am").unwrap();
                        let a_tagged = b.build_int_compare(IntPredicate::EQ, a_masked, ci64(ctx, NANBOX_SIG), "at").unwrap();
                        let b_masked = b.build_and(bv, ci64(ctx, NANBOX_SIG), "bm").unwrap();
                        let b_tagged = b.build_int_compare(IntPredicate::EQ, b_masked, ci64(ctx, NANBOX_SIG), "bt").unwrap();
                        let both_tagged = b.build_and(a_tagged, b_tagged, "bt2").unwrap();

                        // Extract tag bits: (v >> 48) & 7
                        let a_tag = b.build_and(b.build_right_shift(av, ci64(ctx, 48), false, "as48").unwrap(), ci64(ctx, 7), "atg").unwrap();
                        let b_tag = b.build_and(b.build_right_shift(bv, ci64(ctx, 48), false, "bs48").unwrap(), ci64(ctx, 7), "btg").unwrap();
                        let a_is_i64 = b.build_int_compare(IntPredicate::EQ, a_tag, ci64(ctx, tag::I64 as u64), "ai").unwrap();
                        let b_is_i64 = b.build_int_compare(IntPredicate::EQ, b_tag, ci64(ctx, tag::I64 as u64), "bi").unwrap();
                        let both_i64 = b.build_and(a_is_i64, b_is_i64, "bi2").unwrap();
                        let both_int = b.build_and(both_tagged, both_i64, "bint").unwrap();

                        let int_bb = ctx.append_basic_block(lf, "iop");
                        let float_bb = ctx.append_basic_block(lf, "fop");
                        let fallback_bb = ctx.append_basic_block(lf, "fbop");
                        let merge_bb = ctx.append_basic_block(lf, "mop");

                        b.build_conditional_branch(both_int, int_bb, float_bb).unwrap();

                        // ── Integer path ──
                        b.position_at_end(int_bb);
                        let ai = ext_i64(b, ctx, av);
                        let bi = ext_i64(b, ctx, bv);
                        let int_result = match fn_name.as_str() {
                            "__add" => tag_i64(b, ctx, b.build_int_add(ai, bi, "ia").unwrap()),
                            "__sub" => tag_i64(b, ctx, b.build_int_sub(ai, bi, "is").unwrap()),
                            "__mul" => tag_i64(b, ctx, b.build_int_mul(ai, bi, "im").unwrap()),
                            "__div" => {
                                let zr = b.build_int_compare(IntPredicate::EQ, bi, i64_t.const_zero(), "dz").unwrap();
                                let sb = b.build_select(zr, i64_t.const_int(1, false), bi, "sb").unwrap().into_int_value();
                                let dv = b.build_int_signed_div(ai, sb, "id").unwrap();
                                let dr = b.build_select(zr, i64_t.const_zero(), dv, "dr").unwrap().into_int_value();
                                tag_i64(b, ctx, dr)
                            }
                            "__mod" | "__rem" => {
                                let zr = b.build_int_compare(IntPredicate::EQ, bi, i64_t.const_zero(), "mz").unwrap();
                                let sb = b.build_select(zr, i64_t.const_int(1, false), bi, "smb").unwrap().into_int_value();
                                let rm = b.build_int_signed_rem(ai, sb, "ir").unwrap();
                                let rr = b.build_select(zr, i64_t.const_zero(), rm, "rr").unwrap().into_int_value();
                                tag_i64(b, ctx, rr)
                            }
                            "__lt" => tag_bool(b, ctx, b.build_int_compare(IntPredicate::SLT, ai, bi, "lt").unwrap()),
                            "__gt" => tag_bool(b, ctx, b.build_int_compare(IntPredicate::SGT, ai, bi, "gt").unwrap()),
                            "__le" => tag_bool(b, ctx, b.build_int_compare(IntPredicate::SLE, ai, bi, "le").unwrap()),
                            "__ge" => tag_bool(b, ctx, b.build_int_compare(IntPredicate::SGE, ai, bi, "ge").unwrap()),
                            _ => tag_i64(b, ctx, b.build_int_add(ai, bi, "ia").unwrap()),
                        };
                        b.build_unconditional_branch(merge_bb).unwrap();
                        let int_exit_bb = b.get_insert_block().unwrap();

                        // ── Float path: both untagged (raw f64) or mixed int/float ──
                        // Check neither operand is a string/array/map (tag 3,4,5)
                        b.position_at_end(float_bb);
                        let a_is_str_arr = b.build_int_compare(IntPredicate::UGE, a_tag, ci64(ctx, tag::STRING as u64), "asa").unwrap();
                        let b_is_str_arr = b.build_int_compare(IntPredicate::UGE, b_tag, ci64(ctx, tag::STRING as u64), "bsa").unwrap();
                        // If untagged (float), tag bits are garbage — check tagged flag first
                        let a_needs_fb = b.build_and(a_tagged, a_is_str_arr, "anf").unwrap();
                        let b_needs_fb = b.build_and(b_tagged, b_is_str_arr, "bnf").unwrap();
                        let any_complex = b.build_or(a_needs_fb, b_needs_fb, "acx").unwrap();
                        b.build_conditional_branch(any_complex, fallback_bb, {
                            let do_float_bb = ctx.append_basic_block(lf, "dfl");
                            do_float_bb
                        }).unwrap();

                        // Actual float computation block (after the check)
                        let do_float_bb = lf.get_last_basic_block().unwrap();
                        b.position_at_end(do_float_bb);
                        // Convert both to f64: untagged values are already IEEE doubles;
                        // tagged i64 values need int→float conversion
                        let af = {
                            let raw_f = to_f64(b, ctx, av);
                            let int_as_f = b.build_signed_int_to_float(ext_i64(b, ctx, av), ctx.f64_type(), "aif").unwrap();
                            b.build_select(a_tagged, int_as_f, raw_f, "af").unwrap().into_float_value()
                        };
                        let bf = {
                            let raw_f = to_f64(b, ctx, bv);
                            let int_as_f = b.build_signed_int_to_float(ext_i64(b, ctx, bv), ctx.f64_type(), "bif").unwrap();
                            b.build_select(b_tagged, int_as_f, raw_f, "bf").unwrap().into_float_value()
                        };
                        let float_result = match fn_name.as_str() {
                            "__add" => from_f64(b, ctx, b.build_float_add(af, bf, "fa").unwrap()),
                            "__sub" => from_f64(b, ctx, b.build_float_sub(af, bf, "fs").unwrap()),
                            "__mul" => from_f64(b, ctx, b.build_float_mul(af, bf, "fm").unwrap()),
                            "__div" => from_f64(b, ctx, b.build_float_div(af, bf, "fd").unwrap()),
                            "__mod" | "__rem" => from_f64(b, ctx, b.build_float_rem(af, bf, "fre").unwrap()),
                            "__lt" => tag_bool(b, ctx, b.build_float_compare(inkwell::FloatPredicate::OLT, af, bf, "flt").unwrap()),
                            "__gt" => tag_bool(b, ctx, b.build_float_compare(inkwell::FloatPredicate::OGT, af, bf, "fgt").unwrap()),
                            "__le" => tag_bool(b, ctx, b.build_float_compare(inkwell::FloatPredicate::OLE, af, bf, "fle").unwrap()),
                            "__ge" => tag_bool(b, ctx, b.build_float_compare(inkwell::FloatPredicate::OGE, af, bf, "fge").unwrap()),
                            _ => from_f64(b, ctx, b.build_float_add(af, bf, "fa").unwrap()),
                        };
                        b.build_unconditional_branch(merge_bb).unwrap();
                        let float_exit_bb = b.get_insert_block().unwrap();

                        // ── Fallback: call C runtime for strings/arrays/maps ──
                        b.position_at_end(fallback_bb);
                        let p0 = unsafe { b.build_gep(i64_t, rc_args_buf, &[i32_t.const_int(0, false)], "p0").unwrap() };
                        b.build_store(p0, av).unwrap();
                        let p1 = unsafe { b.build_gep(i64_t, rc_args_buf, &[i32_t.const_int(1, false)], "p1").unwrap() };
                        b.build_store(p1, bv).unwrap();
                        let fb_r = if let Some(rid) = runtime_id(fn_name) {
                            b.build_call(rt["__magi_runtime_call_id"], &[i32_t.const_int(rid as u64, false).into(), i32_t.const_int(2, false).into(), rc_args_buf.into()], "fr").unwrap()
                        } else {
                            let np = str_ptr(b, ir, str_cache, *name);
                            b.build_call(rt["__magi_runtime_call"], &[np.into(), i32_t.const_int(2, false).into(), rc_args_buf.into()], "fr").unwrap()
                        };
                        let fb_val = fb_r.try_as_basic_value().left().unwrap().into_int_value();
                        b.build_unconditional_branch(merge_bb).unwrap();
                        let fb_exit_bb = b.get_insert_block().unwrap();

                        // ── Merge with phi ──
                        b.position_at_end(merge_bb);
                        let phi = b.build_phi(i64_t, "opr").unwrap();
                        phi.add_incoming(&[(&int_result, int_exit_bb), (&float_result, float_exit_bb), (&fb_val, fb_exit_bb)]);
                        stack.push(phi.as_basic_value().into_int_value());
                    } else {
                        let ac = *arg_count;
                        let mut args: Vec<IntValue> = (0..ac).map(|_| stack.pop().unwrap_or(cnull(ctx))).collect();
                        args.reverse();
                        for (i, a) in args.iter().enumerate() {
                            let p = unsafe { b.build_gep(i64_t, rc_args_buf, &[i32_t.const_int(i as u64, false)], "ap").unwrap() };
                            b.build_store(p, *a).unwrap();
                        }
                        let r = if let Some(rid) = runtime_id(fn_name) {
                            b.build_call(rt["__magi_runtime_call_id"], &[i32_t.const_int(rid as u64, false).into(), i32_t.const_int(ac as u64, false).into(), rc_args_buf.into()], "rc").unwrap()
                        } else {
                            let np = str_ptr(b, ir, str_cache, *name);
                            b.build_call(rt["__magi_runtime_call"], &[np.into(), i32_t.const_int(ac as u64, false).into(), rc_args_buf.into()], "rc").unwrap()
                        };
                        stack.push(r.try_as_basic_value().left().unwrap().into_int_value());
                    }
                }
            }

            // ── Raw (untagged) loop counter operations ────
            Instruction::PushRawI64(n) => {
                stack.push(i64_t.const_int(*n as u64, false));
            }
            Instruction::RawI64Add => {
                let bv = stack.pop().unwrap_or(i64_t.const_zero());
                let av = stack.pop().unwrap_or(i64_t.const_zero());
                stack.push(b.build_int_add(av, bv, "radd").unwrap());
            }
            Instruction::RawI64Ge => {
                let bv = stack.pop().unwrap_or(i64_t.const_zero());
                let av = stack.pop().unwrap_or(i64_t.const_zero());
                let c = b.build_int_compare(IntPredicate::SGE, av, bv, "rge").unwrap();
                stack.push(b.build_int_z_extend(c, i64_t, "rgex").unwrap());
            }
            Instruction::RawBrIf(depth) => {
                let c = stack.pop().unwrap_or(i64_t.const_zero());
                // Raw boolean: nonzero = true, no truthy extraction needed.
                let cb = b.build_int_compare(IntPredicate::NE, c, i64_t.const_zero(), "rbnz").unwrap();
                let idx = cf.len().saturating_sub(1 + *depth as usize);
                let cont = ctx.append_basic_block(lf, "rbc");
                if let Some(e) = cf.get(idx) {
                    let t = if e.is_loop { e.branch_target } else { e.merge_block };
                    if !terminated(b) { b.build_conditional_branch(cb, t, cont).unwrap(); }
                }
                b.position_at_end(cont);
            }
            Instruction::RawArrayLen => {
                // Inline array length extraction: load arr->len as raw i32, extend to i64.
                let arr_val = stack.pop().unwrap_or(cnull(ctx));

                // Extract MagiArray* from payload bits.
                let ptr_raw = b.build_and(arr_val, ci64(ctx, PAYLOAD_MASK_U64), "rlpr").unwrap();
                let arr_ptr = b.build_int_to_ptr(
                    ptr_raw, ctx.ptr_type(AddressSpace::default()), "rlap",
                ).unwrap();

                // Load len at byte offset 8 (struct: {i64* data, i32 len, i32 cap}).
                let len_byte_ptr = unsafe {
                    b.build_gep(ctx.i8_type(), arr_ptr,
                        &[i32_t.const_int(8, false)], "rllbp").unwrap()
                };
                let len_i32 = b.build_load(i32_t, len_byte_ptr, "rllen")
                    .unwrap().into_int_value();
                let len_i64 = b.build_int_z_extend(len_i32, i64_t, "rll64").unwrap();
                stack.push(len_i64);
            }
            Instruction::RawArrayGet => {
                // Pop raw i64 index + tagged array, push tagged element.
                let raw_idx = stack.pop().unwrap_or(i64_t.const_zero());
                let arr_val = stack.pop().unwrap_or(cnull(ctx));

                // Check if arr_val is a NaN-boxed TAG_ARRAY: (v >> 48) & 7 == 4
                let arr_tag = b.build_and(
                    b.build_right_shift(arr_val, ci64(ctx, 48), false, "rgas48").unwrap(),
                    ci64(ctx, 7), "rgatg",
                ).unwrap();
                let is_array = b.build_int_compare(
                    IntPredicate::EQ, arr_tag, ci64(ctx, tag::ARRAY as u64), "rgia",
                ).unwrap();

                let fast_bb = ctx.append_basic_block(lf, "rga_fast");
                let slow_bb = ctx.append_basic_block(lf, "rga_slow");
                let merge_bb = ctx.append_basic_block(lf, "rga_merge");

                b.build_conditional_branch(is_array, fast_bb, slow_bb).unwrap();

                // ── Fast path: inline array access with raw index ──
                b.position_at_end(fast_bb);

                let ptr_raw = b.build_and(arr_val, ci64(ctx, PAYLOAD_MASK_U64), "rgapr").unwrap();
                let arr_ptr = b.build_int_to_ptr(
                    ptr_raw, ctx.ptr_type(AddressSpace::default()), "rgaap",
                ).unwrap();

                // Load cap at byte offset 12
                let cap_byte_ptr = unsafe {
                    b.build_gep(ctx.i8_type(), arr_ptr,
                        &[i32_t.const_int(12, false)], "rgacbp").unwrap()
                };
                let cap_val = b.build_load(i32_t, cap_byte_ptr, "rgacap")
                    .unwrap().into_int_value();

                let is_regular = b.build_int_compare(
                    IntPredicate::NE, cap_val,
                    i32_t.const_int(-1i32 as u32 as u64, false), "rgair",
                ).unwrap();

                // Truncate raw i64 index to i32 for bounds check
                let idx_i32 = b.build_int_truncate(raw_idx, i32_t, "rgai32").unwrap();

                // Load len at byte offset 8
                let len_byte_ptr = unsafe {
                    b.build_gep(ctx.i8_type(), arr_ptr,
                        &[i32_t.const_int(8, false)], "rgalbp").unwrap()
                };
                let len_val = b.build_load(i32_t, len_byte_ptr, "rgalen")
                    .unwrap().into_int_value();

                // Bounds check: 0 <= idx < len
                let ge_zero = b.build_int_compare(
                    IntPredicate::SGE, idx_i32, i32_t.const_zero(), "rgagz",
                ).unwrap();
                let lt_len = b.build_int_compare(
                    IntPredicate::SLT, idx_i32, len_val, "rgall",
                ).unwrap();
                let in_bounds = b.build_and(ge_zero, lt_len, "rgaib").unwrap();
                let can_inline = b.build_and(is_regular, in_bounds, "rgaci").unwrap();

                let inline_bb = ctx.append_basic_block(lf, "rga_inline");
                let oob_bb = ctx.append_basic_block(lf, "rga_oob");
                b.build_conditional_branch(can_inline, inline_bb, oob_bb).unwrap();

                // ── Inline load: arr->data[idx] ──
                b.position_at_end(inline_bb);
                let data_ptr = b.build_load(
                    ctx.ptr_type(AddressSpace::default()), arr_ptr, "rgadp",
                ).unwrap().into_pointer_value();
                let idx_u64 = b.build_int_z_extend(idx_i32, i64_t, "rgaiu64").unwrap();
                let elem_ptr = unsafe {
                    b.build_gep(i64_t, data_ptr, &[idx_u64], "rgaep").unwrap()
                };
                let fast_result = b.build_load(i64_t, elem_ptr, "rgafr")
                    .unwrap().into_int_value();
                b.build_unconditional_branch(merge_bb).unwrap();

                // ── OOB: return null ──
                b.position_at_end(oob_bb);
                let oob_result = cnull(ctx);
                b.build_unconditional_branch(merge_bb).unwrap();

                // ── Slow path: maps, byte arrays → call C runtime ──
                // Tag the raw index before calling C runtime (expects tagged values)
                b.position_at_end(slow_bb);
                let tagged_idx = tag_i64(b, ctx, raw_idx);
                let slow_r = b.build_call(
                    rt["__magi_array_get"],
                    &[arr_val.into(), tagged_idx.into()], "rgasg",
                ).unwrap();
                let slow_result = slow_r.try_as_basic_value().left()
                    .unwrap().into_int_value();
                b.build_unconditional_branch(merge_bb).unwrap();

                // ── Merge ──
                b.position_at_end(merge_bb);
                let phi = b.build_phi(i64_t, "rga_r").unwrap();
                phi.add_incoming(&[(&fast_result, inline_bb), (&oob_result, oob_bb), (&slow_result, slow_bb)]);
                stack.push(phi.as_basic_value().into_int_value());
            }
        }
    }

    // Implicit return
    if !terminated(b) {
        let rv = stack.pop().unwrap_or(cnull(ctx));
        b.build_return(Some(&rv)).unwrap();
    }

    Ok(())
}
