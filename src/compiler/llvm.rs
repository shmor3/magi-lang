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
    target_triple: Option<&str>,
    opt_level: u8,
    output_path: &str,
) -> Result<(), String> {
    let program = crate::syntax::parser::parse_v2(source)
        .map_err(|e| format!("parse error: {}", e.message))?;

    let mut compiler = super::Compiler::new();
    let ir_mod = compiler.compile(&program).map_err(|e| format!("{}", e))?;

    emit_native(&ir_mod, target_triple, opt_level, output_path)
}

/// Core LLVM pipeline: IR module → object file → linked binary.
/// All LLVM objects live in this function's scope to satisfy lifetime requirements.
fn emit_native(
    ir_mod: &IrModule,
    target_triple: Option<&str>,
    opt_level: u8,
    output_path: &str,
) -> Result<(), String> {
    let ctx = Context::create();
    let module = ctx.create_module("magi_program");
    let b = ctx.create_builder();

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

    // ── Globals ─────────────────────────────────────────────
    let mut globals: Vec<PointerValue> = Vec::new();
    for g in &ir_mod.globals {
        let gv = module.add_global(i64_t, None, &g.name);
        gv.set_initializer(&i64_t.const_int(NULL_TAG, false));
        globals.push(gv.as_pointer_value());
    }

    // ── Declare functions ───────────────────────────────────
    let mut fns: HashMap<String, FunctionValue> = HashMap::new();
    for func in &ir_mod.functions {
        let params: Vec<BasicMetadataTypeEnum> = (0..func.param_count).map(|_| i64_t.into()).collect();
        let ft = i64_t.fn_type(&params, false);
        fns.insert(func.name.clone(), module.add_function(&func.name, ft, None));
    }

    // ── Compile function bodies ─────────────────────────────
    let mut str_cache: HashMap<u32, PointerValue> = HashMap::new();

    for func in &ir_mod.functions {
        let lf = *fns.get(&func.name).unwrap();
        compile_fn(&ctx, &module, &b, ir_mod, &fns, &rt, &globals, &mut str_cache, func, lf)?;
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
        let original = *fns.get(&func.name).unwrap();
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

    // ── main() entry point ──────────────────────────────────
    if let Some(mf) = fns.get("__main").copied() {
        let main = module.add_function("main", i32_t.fn_type(&[], false), None);
        let entry = ctx.append_basic_block(main, "entry");
        b.position_at_end(entry);
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
    let triple = target_triple.map(TargetTriple::create).unwrap_or_else(TargetMachine::get_default_triple);
    let target = Target::from_triple(&triple).map_err(|e| format!("invalid target: {}", e))?;
    let machine = target
        .create_target_machine(&triple, "generic", "", opt, RelocMode::PIC, CodeModel::Default)
        .ok_or("failed to create target machine")?;

    let obj_path = format!("{}.o", output_path);
    machine.write_to_file(&module, FileType::Object, std::path::Path::new(&obj_path))
        .map_err(|e| format!("failed to write object: {}", e))?;

    let rt_path = format!("{}.magi_rt.c", output_path);
    std::fs::write(&rt_path, RUNTIME_C_SOURCE).map_err(|e| format!("write runtime: {}", e))?;

    let status = std::process::Command::new("cc")
        .args([&obj_path, &rt_path, "-o", output_path, "-lm", "-O2"])
        .status()
        .map_err(|e| format!("linker: {}", e))?;

    let _ = std::fs::remove_file(&obj_path);
    let _ = std::fs::remove_file(&rt_path);

    if !status.success() { return Err("linking failed".into()); }
    Ok(())
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

// ── Function body compilation ───────────────────────────────

#[allow(clippy::too_many_arguments)]
fn compile_fn<'ctx>(
    ctx: &'ctx Context,
    module: &Module<'ctx>,
    b: &Builder<'ctx>,
    ir: &IrModule,
    fns: &HashMap<String, FunctionValue<'ctx>>,
    rt: &HashMap<&str, FunctionValue<'ctx>>,
    globals: &[PointerValue<'ctx>],
    str_cache: &mut HashMap<u32, PointerValue<'ctx>>,
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
            Instruction::I64Div => { let bv = ext_i64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); let av = ext_i64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); stack.push(tag_i64(b,ctx,b.build_int_signed_div(av,bv,"d").unwrap())); }
            Instruction::I64Rem => { let bv = ext_i64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); let av = ext_i64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); stack.push(tag_i64(b,ctx,b.build_int_signed_rem(av,bv,"r").unwrap())); }
            Instruction::I64Neg => { let av = ext_i64(b,ctx,stack.pop().unwrap_or(cnull(ctx))); stack.push(tag_i64(b,ctx,b.build_int_neg(av,"n").unwrap())); }

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
                if let Some(tlf) = fns.get(&tf.name).copied() {
                    let ac = tf.param_count as usize;
                    let mut args: Vec<BasicMetadataValueEnum> = Vec::new();
                    for _ in 0..ac { args.push(stack.pop().unwrap_or(cnull(ctx)).into()); }
                    args.reverse();
                    let r = b.build_call(tlf, &args, "c").unwrap();
                    if let Some(v) = r.try_as_basic_value().left() { stack.push(v.into_int_value()); }
                }
            }
            Instruction::CallIndirect(type_idx) => {
                // Indirect call: function index is on the stack, then args
                // type_idx tells us the arity from the IR type table
                let fn_idx_val = stack.pop().unwrap_or(cnull(ctx));
                let arity = ir.functions.get(*type_idx as usize).map(|f| f.param_count).unwrap_or(0) as usize;
                let mut call_args: Vec<IntValue> = (0..arity).map(|_| stack.pop().unwrap_or(cnull(ctx))).collect();
                call_args.reverse();

                // Build args array for __magi_call_fn
                let args_alloca = b.build_array_alloca(i64_t, i32_t.const_int(std::cmp::max(arity, 1) as u64, false), "ia").unwrap();
                for (i, a) in call_args.iter().enumerate() {
                    let p = unsafe { b.build_gep(i64_t, args_alloca, &[i32_t.const_int(i as u64, false)], "iap").unwrap() };
                    b.build_store(p, *a).unwrap();
                }

                // Declare __magi_call_fn if needed
                let call_fn = module.get_function("__magi_call_fn").unwrap_or_else(|| {
                    let ft = i64_t.fn_type(&[i64_t.into(), i32_t.into(), ctx.ptr_type(AddressSpace::default()).into()], false);
                    module.add_function("__magi_call_fn", ft, None)
                });
                let result = b.build_call(call_fn, &[fn_idx_val.into(), i32_t.const_int(arity as u64, false).into(), args_alloca.into()], "ic").unwrap();
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
                let idx = stack.pop().unwrap_or(cnull(ctx));
                let arr = stack.pop().unwrap_or(cnull(ctx));
                let r = b.build_call(rt["__magi_array_get"], &[arr.into(), idx.into()], "ag").unwrap();
                stack.push(r.try_as_basic_value().left().unwrap().into_int_value());
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
                let np = str_ptr(b, ir, str_cache, *name);
                let ac = *arg_count;
                let aa = b.build_array_alloca(i64_t, i32_t.const_int(std::cmp::max(ac,1) as u64, false), "ra").unwrap();
                let mut args: Vec<IntValue> = (0..ac).map(|_| stack.pop().unwrap_or(cnull(ctx))).collect();
                args.reverse();
                for (i, a) in args.iter().enumerate() {
                    let p = unsafe { b.build_gep(i64_t, aa, &[i32_t.const_int(i as u64, false)], "ap").unwrap() };
                    b.build_store(p, *a).unwrap();
                }
                let r = b.build_call(rt["__magi_runtime_call"], &[np.into(), i32_t.const_int(ac as u64, false).into(), aa.into()], "rc").unwrap();
                stack.push(r.try_as_basic_value().left().unwrap().into_int_value());
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
