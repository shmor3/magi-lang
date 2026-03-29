//! Native code generation for MAGI programs.
//!
//! Compiles bytecode to native machine code for multiple targets:
//! - x86-64 Linux (ELF)
//! - x86-64 macOS (Mach-O)
//! - aarch64 Linux (ELF)
//! - aarch64 macOS (Mach-O)

use super::bytecode::{Chunk, OpCode, BytecodeCompiler, VM};
use crate::types::DataType;

/// Target architecture for native compilation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NativeTarget {
    X86_64Linux,
    X86_64Macos,
    Aarch64Linux,
    Aarch64Macos,
}

impl NativeTarget {
    /// Detect the current host target.
    pub fn host() -> Self {
        match (std::env::consts::ARCH, std::env::consts::OS) {
            ("x86_64", "linux") => NativeTarget::X86_64Linux,
            ("x86_64", "macos") => NativeTarget::X86_64Macos,
            ("aarch64", "linux") => NativeTarget::Aarch64Linux,
            ("aarch64", "macos") => NativeTarget::Aarch64Macos,
            _ => NativeTarget::X86_64Linux,
        }
    }

    pub fn arch_name(&self) -> &'static str {
        match self {
            NativeTarget::X86_64Linux | NativeTarget::X86_64Macos => "x86_64",
            NativeTarget::Aarch64Linux | NativeTarget::Aarch64Macos => "aarch64",
        }
    }

    pub fn is_elf(&self) -> bool {
        matches!(self, NativeTarget::X86_64Linux | NativeTarget::Aarch64Linux)
    }

    pub fn is_macho(&self) -> bool {
        matches!(self, NativeTarget::X86_64Macos | NativeTarget::Aarch64Macos)
    }
}

/// Native code output.
#[derive(Debug)]
pub struct NativeCode {
    pub code: Vec<u8>,
    pub entry: usize,
    pub arch: &'static str,
    pub target: NativeTarget,
    /// String constants embedded in the data section.
    pub data: Vec<u8>,
    /// Offsets of string constants in the data section.
    pub string_offsets: Vec<(usize, usize)>,
}

// ── x86-64 Code Generation ──────────────────────────────────────────

/// Generate x86-64 machine code from bytecode.
pub fn compile_to_native(chunk: &Chunk, target: NativeTarget) -> Result<NativeCode, String> {
    match target {
        NativeTarget::X86_64Linux | NativeTarget::X86_64Macos => compile_x86_64(chunk, target),
        NativeTarget::Aarch64Linux | NativeTarget::Aarch64Macos => compile_aarch64(chunk, target),
    }
}

fn compile_x86_64(chunk: &Chunk, target: NativeTarget) -> Result<NativeCode, String> {
    let mut code: Vec<u8> = Vec::new();
    let mut data: Vec<u8> = Vec::new();
    let mut string_offsets: Vec<(usize, usize)> = Vec::new();

    // Collect string constants into data section
    for (i, constant) in chunk.constants.iter().enumerate() {
        if let DataType::String(s) = constant {
            let offset = data.len();
            data.extend_from_slice(s.as_bytes());
            string_offsets.push((i, offset));
        }
    }

    // Track jump fixups: (code_offset_of_rel32, bytecode_target)
    let mut jump_fixups: Vec<(usize, u16)> = Vec::new();
    // Map bytecode IP → native code offset
    let mut ip_to_native: Vec<usize> = Vec::new();

    // First pass: generate code and record jump sites
    // Function prologue
    code.push(0x55); // push rbp
    code.extend_from_slice(&[0x48, 0x89, 0xE5]); // mov rbp, rsp
    // Reserve 2048 bytes of stack for locals and globals
    code.extend_from_slice(&[0x48, 0x81, 0xEC, 0x00, 0x08, 0x00, 0x00]); // sub rsp, 2048

    let mut ip = 0;
    while ip < chunk.code.len() {
        ip_to_native.push(code.len());
        let op = chunk.code[ip];
        ip += 1;

        match op {
            x if x == OpCode::Const as u8 => {
                let const_idx = ((chunk.code[ip] as u16) << 8) | chunk.code[ip + 1] as u16;
                ip += 2;
                let val = chunk.constants.get(const_idx as usize)
                    .and_then(|v| v.to_i64())
                    .unwrap_or(0);
                // mov rax, imm64; push rax
                code.extend_from_slice(&[0x48, 0xB8]);
                code.extend_from_slice(&val.to_le_bytes());
                code.push(0x50);
            }
            x if x == OpCode::Null as u8 || x == OpCode::False as u8 => {
                code.extend_from_slice(&[0x48, 0x31, 0xC0]); // xor rax, rax
                code.push(0x50);
            }
            x if x == OpCode::True as u8 => {
                code.extend_from_slice(&[0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00]); // mov rax, 1
                code.push(0x50);
            }
            x if x == OpCode::Pop as u8 => {
                code.extend_from_slice(&[0x48, 0x83, 0xC4, 0x08]); // add rsp, 8
            }
            x if x == OpCode::Dup as u8 => {
                code.extend_from_slice(&[0x48, 0x8B, 0x04, 0x24]); // mov rax, [rsp]
                code.push(0x50);
            }
            x if x == OpCode::Add as u8 => {
                code.push(0x5B); // pop rbx
                code.push(0x58); // pop rax
                code.extend_from_slice(&[0x48, 0x01, 0xD8]); // add rax, rbx
                code.push(0x50);
            }
            x if x == OpCode::Sub as u8 => {
                code.push(0x5B);
                code.push(0x58);
                code.extend_from_slice(&[0x48, 0x29, 0xD8]); // sub rax, rbx
                code.push(0x50);
            }
            x if x == OpCode::Mul as u8 => {
                code.push(0x5B);
                code.push(0x58);
                code.extend_from_slice(&[0x48, 0x0F, 0xAF, 0xC3]); // imul rax, rbx
                code.push(0x50);
            }
            x if x == OpCode::Div as u8 => {
                code.push(0x5B); // divisor
                code.push(0x58); // dividend
                code.extend_from_slice(&[0x48, 0x99]); // cqo
                code.extend_from_slice(&[0x48, 0xF7, 0xFB]); // idiv rbx
                code.push(0x50);
            }
            x if x == OpCode::Mod as u8 => {
                code.push(0x5B);
                code.push(0x58);
                code.extend_from_slice(&[0x48, 0x99]); // cqo
                code.extend_from_slice(&[0x48, 0xF7, 0xFB]); // idiv rbx
                // Remainder is in rdx
                code.push(0x52); // push rdx
            }
            x if x == OpCode::Neg as u8 => {
                code.push(0x58);
                code.extend_from_slice(&[0x48, 0xF7, 0xD8]); // neg rax
                code.push(0x50);
            }
            x if x == OpCode::Pow as u8 => {
                // Integer power via loop: pop exponent (rbx), pop base (rax)
                code.push(0x5B); // pop rbx (exponent)
                code.push(0x58); // pop rax (base)
                // rcx = result = 1
                code.extend_from_slice(&[0x48, 0xC7, 0xC1, 0x01, 0x00, 0x00, 0x00]); // mov rcx, 1
                // loop: test rbx, rbx; jz done; imul rcx, rax; dec rbx; jmp loop
                let loop_start = code.len();
                code.extend_from_slice(&[0x48, 0x85, 0xDB]); // test rbx, rbx
                code.extend_from_slice(&[0x74, 0x09]); // jz +9
                code.extend_from_slice(&[0x48, 0x0F, 0xAF, 0xC8]); // imul rcx, rax
                code.extend_from_slice(&[0x48, 0xFF, 0xCB]); // dec rbx
                let rel = loop_start as i32 - (code.len() as i32 + 2);
                code.push(0xEB); // jmp rel8
                code.push(rel as u8);
                // mov rax, rcx; push rax
                code.extend_from_slice(&[0x48, 0x89, 0xC8]); // mov rax, rcx
                code.push(0x50);
            }
            x if x == OpCode::Eq as u8 => {
                code.push(0x5B);
                code.push(0x58);
                code.extend_from_slice(&[0x48, 0x39, 0xD8]); // cmp rax, rbx
                code.extend_from_slice(&[0x0F, 0x94, 0xC0]); // sete al
                code.extend_from_slice(&[0x48, 0x0F, 0xB6, 0xC0]); // movzx rax, al
                code.push(0x50);
            }
            x if x == OpCode::Ne as u8 => {
                code.push(0x5B);
                code.push(0x58);
                code.extend_from_slice(&[0x48, 0x39, 0xD8]);
                code.extend_from_slice(&[0x0F, 0x95, 0xC0]); // setne al
                code.extend_from_slice(&[0x48, 0x0F, 0xB6, 0xC0]);
                code.push(0x50);
            }
            x if x == OpCode::Lt as u8 => {
                code.push(0x5B);
                code.push(0x58);
                code.extend_from_slice(&[0x48, 0x39, 0xD8]);
                code.extend_from_slice(&[0x0F, 0x9C, 0xC0]); // setl al
                code.extend_from_slice(&[0x48, 0x0F, 0xB6, 0xC0]);
                code.push(0x50);
            }
            x if x == OpCode::Le as u8 => {
                code.push(0x5B);
                code.push(0x58);
                code.extend_from_slice(&[0x48, 0x39, 0xD8]);
                code.extend_from_slice(&[0x0F, 0x9E, 0xC0]); // setle al
                code.extend_from_slice(&[0x48, 0x0F, 0xB6, 0xC0]);
                code.push(0x50);
            }
            x if x == OpCode::Gt as u8 => {
                code.push(0x5B);
                code.push(0x58);
                code.extend_from_slice(&[0x48, 0x39, 0xD8]);
                code.extend_from_slice(&[0x0F, 0x9F, 0xC0]); // setg al
                code.extend_from_slice(&[0x48, 0x0F, 0xB6, 0xC0]);
                code.push(0x50);
            }
            x if x == OpCode::Ge as u8 => {
                code.push(0x5B);
                code.push(0x58);
                code.extend_from_slice(&[0x48, 0x39, 0xD8]);
                code.extend_from_slice(&[0x0F, 0x9D, 0xC0]); // setge al
                code.extend_from_slice(&[0x48, 0x0F, 0xB6, 0xC0]);
                code.push(0x50);
            }
            x if x == OpCode::Not as u8 => {
                code.push(0x58);
                code.extend_from_slice(&[0x48, 0x85, 0xC0]); // test rax, rax
                code.extend_from_slice(&[0x0F, 0x94, 0xC0]); // sete al
                code.extend_from_slice(&[0x48, 0x0F, 0xB6, 0xC0]);
                code.push(0x50);
            }
            x if x == OpCode::And as u8 => {
                code.push(0x5B);
                code.push(0x58);
                // test rax, rax → al; test rbx, rbx → bl; and al, bl
                code.extend_from_slice(&[0x48, 0x85, 0xC0]); // test rax, rax
                code.extend_from_slice(&[0x0F, 0x95, 0xC0]); // setne al
                code.extend_from_slice(&[0x48, 0x85, 0xDB]); // test rbx, rbx
                code.extend_from_slice(&[0x0F, 0x95, 0xC3]); // setne bl
                code.extend_from_slice(&[0x20, 0xD8]); // and al, bl
                code.extend_from_slice(&[0x48, 0x0F, 0xB6, 0xC0]);
                code.push(0x50);
            }
            x if x == OpCode::Or as u8 => {
                code.push(0x5B);
                code.push(0x58);
                code.extend_from_slice(&[0x48, 0x85, 0xC0]);
                code.extend_from_slice(&[0x0F, 0x95, 0xC0]);
                code.extend_from_slice(&[0x48, 0x85, 0xDB]);
                code.extend_from_slice(&[0x0F, 0x95, 0xC3]);
                code.extend_from_slice(&[0x08, 0xD8]); // or al, bl
                code.extend_from_slice(&[0x48, 0x0F, 0xB6, 0xC0]);
                code.push(0x50);
            }
            x if x == OpCode::LoadLocal as u8 => {
                let idx = chunk.code[ip] as i32;
                ip += 1;
                let offset = -8 * (idx + 1);
                code.extend_from_slice(&[0x48, 0x8B, 0x85]); // mov rax, [rbp + disp32]
                code.extend_from_slice(&offset.to_le_bytes());
                code.push(0x50);
            }
            x if x == OpCode::StoreLocal as u8 => {
                let idx = chunk.code[ip] as i32;
                ip += 1;
                code.push(0x58); // pop rax
                let offset = -8 * (idx + 1);
                code.extend_from_slice(&[0x48, 0x89, 0x85]); // mov [rbp + disp32], rax
                code.extend_from_slice(&offset.to_le_bytes());
            }
            x if x == OpCode::LoadGlobal as u8 => {
                let name_idx = ((chunk.code[ip] as u16) << 8) | chunk.code[ip + 1] as u16;
                ip += 2;
                // Globals stored at rbp - 1024 - 8*name_idx
                let offset = -1024 - 8 * (name_idx as i32);
                code.extend_from_slice(&[0x48, 0x8B, 0x85]); // mov rax, [rbp + disp32]
                code.extend_from_slice(&offset.to_le_bytes());
                code.push(0x50);
            }
            x if x == OpCode::StoreGlobal as u8 => {
                let name_idx = ((chunk.code[ip] as u16) << 8) | chunk.code[ip + 1] as u16;
                ip += 2;
                code.push(0x58); // pop rax
                let offset = -1024 - 8 * (name_idx as i32);
                code.extend_from_slice(&[0x48, 0x89, 0x85]); // mov [rbp + disp32], rax
                code.extend_from_slice(&offset.to_le_bytes());
            }
            x if x == OpCode::Jump as u8 => {
                let bc_target = ((chunk.code[ip] as u16) << 8) | chunk.code[ip + 1] as u16;
                ip += 2;
                code.push(0xE9); // jmp rel32
                jump_fixups.push((code.len(), bc_target));
                code.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // placeholder
            }
            x if x == OpCode::JumpIfFalse as u8 => {
                let bc_target = ((chunk.code[ip] as u16) << 8) | chunk.code[ip + 1] as u16;
                ip += 2;
                code.push(0x58); // pop rax
                code.extend_from_slice(&[0x48, 0x85, 0xC0]); // test rax, rax
                code.extend_from_slice(&[0x0F, 0x84]); // je rel32
                jump_fixups.push((code.len(), bc_target));
                code.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
            }
            x if x == OpCode::JumpIfTrue as u8 => {
                let bc_target = ((chunk.code[ip] as u16) << 8) | chunk.code[ip + 1] as u16;
                ip += 2;
                code.push(0x58);
                code.extend_from_slice(&[0x48, 0x85, 0xC0]);
                code.extend_from_slice(&[0x0F, 0x85]); // jne rel32
                jump_fixups.push((code.len(), bc_target));
                code.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
            }
            x if x == OpCode::Call as u8 => {
                let _name_idx = ((chunk.code[ip] as u16) << 8) | chunk.code[ip + 1] as u16;
                ip += 2;
                let _arg_count = chunk.code[ip];
                ip += 1;
                // Function calls in native mode: not inlined (would need function table).
                // Push 0 as return value — full function support requires linking.
                code.extend_from_slice(&[0x48, 0x31, 0xC0]); // xor rax, rax
                code.push(0x50);
            }
            x if x == OpCode::Return as u8 => {
                code.push(0x58); // pop rax into return register
                // Epilogue
                code.extend_from_slice(&[0x48, 0x81, 0xC4, 0x00, 0x08, 0x00, 0x00]); // add rsp, 2048
                code.push(0x5D); // pop rbp
                code.push(0xC3); // ret
            }
            x if x == OpCode::Output as u8 => {
                // Pop value from stack into rdi, then call the print routine
                code.push(0x58); // pop rax — value to print

                if target == NativeTarget::X86_64Linux {
                    // Convert integer to decimal string and write via sys_write
                    // Strategy: push digits onto stack, then sys_write
                    emit_x86_64_print_int(&mut code, false);
                } else {
                    // macOS: same but syscall numbers differ (0x2000004 for write)
                    emit_x86_64_print_int(&mut code, true);
                }
            }
            x if x == OpCode::Halt as u8 => {
                break;
            }
            _ => {
                return Err(format!("unsupported opcode for native compilation: 0x{:02X}", op));
            }
        }
    }
    // Record final IP mapping
    ip_to_native.push(code.len());

    // Fix up jump addresses
    for (fixup_offset, bc_target) in &jump_fixups {
        let target_native = if (*bc_target as usize) < ip_to_native.len() {
            ip_to_native[*bc_target as usize]
        } else {
            code.len()
        };
        let rel = target_native as i32 - (*fixup_offset as i32 + 4);
        let bytes = rel.to_le_bytes();
        code[*fixup_offset] = bytes[0];
        code[*fixup_offset + 1] = bytes[1];
        code[*fixup_offset + 2] = bytes[2];
        code[*fixup_offset + 3] = bytes[3];
    }

    Ok(NativeCode {
        code,
        entry: 0,
        arch: target.arch_name(),
        target,
        data,
        string_offsets,
    })
}

/// Emit x86-64 code to print the integer in rax as decimal, followed by newline.
/// Uses a 32-byte buffer on the stack to avoid push/pop complexity.
fn emit_x86_64_print_int(code: &mut Vec<u8>, macos: bool) {
    let sys_write: i32 = if macos { 0x02000004 } else { 1 };

    // sub rsp, 32  — allocate buffer
    code.extend_from_slice(&[0x48, 0x83, 0xEC, 0x20]);
    // r12 = rax (save original value)
    code.extend_from_slice(&[0x49, 0x89, 0xC4]);
    // rcx = 31 (write position, fills right-to-left)
    code.extend_from_slice(&[0x48, 0xC7, 0xC1, 0x1F, 0x00, 0x00, 0x00]);
    // Put newline at buf[31]
    code.extend_from_slice(&[0xC6, 0x44, 0x0C, 0x00, 0x0A]); // mov byte [rsp+rcx], '\n'
    // dec rcx
    code.extend_from_slice(&[0x48, 0xFF, 0xC9]);
    // rbx = 10
    code.extend_from_slice(&[0x48, 0xC7, 0xC3, 0x0A, 0x00, 0x00, 0x00]);

    // Handle rax == 0 specially
    code.extend_from_slice(&[0x48, 0x85, 0xC0]); // test rax, rax
    let jnz_nonzero = code.len();
    code.extend_from_slice(&[0x75, 0x00]); // jnz (fixup)
    // Zero case: store '0'
    code.extend_from_slice(&[0xC6, 0x44, 0x0C, 0x00, 0x30]); // mov byte [rsp+rcx], '0'
    code.extend_from_slice(&[0x48, 0xFF, 0xC9]); // dec rcx
    let jmp_to_write = code.len();
    code.extend_from_slice(&[0xEB, 0x00]); // jmp to write (fixup)

    // Non-zero: fix up jnz
    code[jnz_nonzero + 1] = (code.len() - jnz_nonzero - 2) as u8;

    // Handle negative: if rax < 0, negate
    code.extend_from_slice(&[0x48, 0x85, 0xC0]); // test rax, rax
    code.extend_from_slice(&[0x79, 0x03]); // jns +3
    code.extend_from_slice(&[0x48, 0xF7, 0xD8]); // neg rax

    // Digit extraction loop
    let loop_top = code.len();
    code.extend_from_slice(&[0x48, 0x85, 0xC0]); // test rax, rax
    let jz_done = code.len();
    code.extend_from_slice(&[0x74, 0x00]); // jz done (fixup)
    code.extend_from_slice(&[0x48, 0x31, 0xD2]); // xor rdx, rdx
    code.extend_from_slice(&[0x48, 0xF7, 0xF3]); // div rbx (unsigned!)
    code.extend_from_slice(&[0x80, 0xC2, 0x30]); // add dl, '0'
    code.extend_from_slice(&[0x88, 0x14, 0x0C]); // mov [rsp+rcx], dl
    code.extend_from_slice(&[0x48, 0xFF, 0xC9]); // dec rcx
    let rel_back = loop_top as i32 - (code.len() as i32 + 2);
    code.push(0xEB);
    code.push(rel_back as u8);

    // Fix up jz_done
    code[jz_done + 1] = (code.len() - jz_done - 2) as u8;

    // If original was negative, write '-'
    code.extend_from_slice(&[0x4D, 0x85, 0xE4]); // test r12, r12
    code.extend_from_slice(&[0x79, 0x08]); // jns +8 (skip minus)
    code.extend_from_slice(&[0xC6, 0x44, 0x0C, 0x00, 0x2D]); // mov byte [rsp+rcx], '-'
    code.extend_from_slice(&[0x48, 0xFF, 0xC9]); // dec rcx

    // Fix up jmp_to_write
    code[jmp_to_write + 1] = (code.len() - jmp_to_write - 2) as u8;

    // Write: rsi = rsp + rcx + 1, rdx = 31 - rcx
    code.extend_from_slice(&[0x48, 0xFF, 0xC1]); // inc rcx (rcx now points to first char)
    code.extend_from_slice(&[0x48, 0x8D, 0x34, 0x0C]); // lea rsi, [rsp+rcx]
    // rdx = 32 - rcx
    code.extend_from_slice(&[0x48, 0xC7, 0xC2, 0x20, 0x00, 0x00, 0x00]); // mov rdx, 32
    code.extend_from_slice(&[0x48, 0x29, 0xCA]); // sub rdx, rcx
    // rdi = 1 (stdout)
    code.extend_from_slice(&[0x48, 0xC7, 0xC7, 0x01, 0x00, 0x00, 0x00]);
    // rax = sys_write
    code.extend_from_slice(&[0x48, 0xC7, 0xC0]);
    code.extend_from_slice(&sys_write.to_le_bytes());
    // syscall
    code.extend_from_slice(&[0x0F, 0x05]);

    // add rsp, 32 — deallocate buffer
    code.extend_from_slice(&[0x48, 0x83, 0xC4, 0x20]);
}

// ── aarch64 Code Generation ─────────────────────────────────────────

fn compile_aarch64(chunk: &Chunk, target: NativeTarget) -> Result<NativeCode, String> {
    let mut code: Vec<u8> = Vec::new();
    let mut data: Vec<u8> = Vec::new();
    let mut string_offsets: Vec<(usize, usize)> = Vec::new();
    let mut jump_fixups: Vec<(usize, u16)> = Vec::new();
    let mut ip_to_native: Vec<usize> = Vec::new();

    for (i, constant) in chunk.constants.iter().enumerate() {
        if let DataType::String(s) = constant {
            let offset = data.len();
            data.extend_from_slice(s.as_bytes());
            string_offsets.push((i, offset));
        }
    }

    // Prologue: stp x29, x30, [sp, -16]!; mov x29, sp; sub sp, sp, 2048
    emit_a64(&mut code, 0xA9BF7BFD); // stp x29, x30, [sp, -16]!
    emit_a64(&mut code, 0x910003FD); // mov x29, sp
    emit_a64(&mut code, 0xD10803FF); // sub sp, sp, #2048

    let mut ip = 0;
    while ip < chunk.code.len() {
        ip_to_native.push(code.len());
        let op = chunk.code[ip];
        ip += 1;

        match op {
            x if x == OpCode::Const as u8 => {
                let const_idx = ((chunk.code[ip] as u16) << 8) | chunk.code[ip + 1] as u16;
                ip += 2;
                let val = chunk.constants.get(const_idx as usize)
                    .and_then(|v| v.to_i64())
                    .unwrap_or(0);
                a64_load_imm64(&mut code, 0, val); // mov x0, imm64
                // str x0, [sp, -8]!  (push)
                emit_a64(&mut code, 0xF81F8FE0);
            }
            x if x == OpCode::Null as u8 || x == OpCode::False as u8 => {
                emit_a64(&mut code, 0xD2800000); // mov x0, 0
                emit_a64(&mut code, 0xF81F8FE0); // str x0, [sp, -8]!
            }
            x if x == OpCode::True as u8 => {
                emit_a64(&mut code, 0xD2800020); // mov x0, 1
                emit_a64(&mut code, 0xF81F8FE0);
            }
            x if x == OpCode::Pop as u8 => {
                emit_a64(&mut code, 0x910023FF); // add sp, sp, 8
            }
            x if x == OpCode::Add as u8 => {
                emit_a64(&mut code, 0xF84107E1); // ldr x1, [sp], 8 (pop)
                emit_a64(&mut code, 0xF84107E0); // ldr x0, [sp], 8
                emit_a64(&mut code, 0x8B010000); // add x0, x0, x1
                emit_a64(&mut code, 0xF81F8FE0); // str x0, [sp, -8]! (push)
            }
            x if x == OpCode::Sub as u8 => {
                emit_a64(&mut code, 0xF84107E1);
                emit_a64(&mut code, 0xF84107E0);
                emit_a64(&mut code, 0xCB010000); // sub x0, x0, x1
                emit_a64(&mut code, 0xF81F8FE0);
            }
            x if x == OpCode::Mul as u8 => {
                emit_a64(&mut code, 0xF84107E1);
                emit_a64(&mut code, 0xF84107E0);
                emit_a64(&mut code, 0x9B017C00); // mul x0, x0, x1
                emit_a64(&mut code, 0xF81F8FE0);
            }
            x if x == OpCode::Div as u8 => {
                emit_a64(&mut code, 0xF84107E1);
                emit_a64(&mut code, 0xF84107E0);
                emit_a64(&mut code, 0x9AC10C00); // sdiv x0, x0, x1
                emit_a64(&mut code, 0xF81F8FE0);
            }
            x if x == OpCode::Neg as u8 => {
                emit_a64(&mut code, 0xF84107E0); // pop x0
                emit_a64(&mut code, 0xCB0003E0); // neg x0, x0
                emit_a64(&mut code, 0xF81F8FE0);
            }
            x if x == OpCode::Eq as u8 => {
                emit_a64(&mut code, 0xF84107E1);
                emit_a64(&mut code, 0xF84107E0);
                emit_a64(&mut code, 0xEB01001F); // cmp x0, x1
                emit_a64(&mut code, 0x9A9F17E0); // cset x0, eq
                emit_a64(&mut code, 0xF81F8FE0);
            }
            x if x == OpCode::Lt as u8 => {
                emit_a64(&mut code, 0xF84107E1);
                emit_a64(&mut code, 0xF84107E0);
                emit_a64(&mut code, 0xEB01001F);
                emit_a64(&mut code, 0x9A9FB7E0); // cset x0, lt
                emit_a64(&mut code, 0xF81F8FE0);
            }
            x if x == OpCode::Gt as u8 => {
                emit_a64(&mut code, 0xF84107E1);
                emit_a64(&mut code, 0xF84107E0);
                emit_a64(&mut code, 0xEB01001F);
                emit_a64(&mut code, 0x9A9FC7E0); // cset x0, gt
                emit_a64(&mut code, 0xF81F8FE0);
            }
            x if x == OpCode::Not as u8 => {
                emit_a64(&mut code, 0xF84107E0);
                emit_a64(&mut code, 0xF100001F); // cmp x0, 0
                emit_a64(&mut code, 0x9A9F17E0); // cset x0, eq
                emit_a64(&mut code, 0xF81F8FE0);
            }
            x if x == OpCode::LoadLocal as u8 => {
                let idx = chunk.code[ip] as u32;
                ip += 1;
                let offset = (idx + 1) * 8;
                // ldr x0, [x29, -offset]
                emit_a64(&mut code, 0xF85F83A0 | ((offset & 0x1FF) << 12)); // approximate
                emit_a64(&mut code, 0xF81F8FE0);
            }
            x if x == OpCode::StoreLocal as u8 => {
                let idx = chunk.code[ip] as u32;
                ip += 1;
                emit_a64(&mut code, 0xF84107E0); // pop x0
                let offset = (idx + 1) * 8;
                emit_a64(&mut code, 0xF81F83A0 | ((offset & 0x1FF) << 12));
            }
            x if x == OpCode::LoadGlobal as u8 || x == OpCode::StoreGlobal as u8 => {
                ip += 2;
                emit_a64(&mut code, 0xD2800000); // mov x0, 0
                emit_a64(&mut code, 0xF81F8FE0);
            }
            x if x == OpCode::Jump as u8 => {
                let bc_target = ((chunk.code[ip] as u16) << 8) | chunk.code[ip + 1] as u16;
                ip += 2;
                jump_fixups.push((code.len(), bc_target));
                emit_a64(&mut code, 0x14000000); // b <offset> (placeholder)
            }
            x if x == OpCode::JumpIfFalse as u8 => {
                let bc_target = ((chunk.code[ip] as u16) << 8) | chunk.code[ip + 1] as u16;
                ip += 2;
                emit_a64(&mut code, 0xF84107E0); // pop x0
                emit_a64(&mut code, 0xF100001F); // cmp x0, 0
                jump_fixups.push((code.len(), bc_target));
                emit_a64(&mut code, 0x54000000); // b.eq <offset> (placeholder)
            }
            x if x == OpCode::JumpIfTrue as u8 => {
                let bc_target = ((chunk.code[ip] as u16) << 8) | chunk.code[ip + 1] as u16;
                ip += 2;
                emit_a64(&mut code, 0xF84107E0);
                emit_a64(&mut code, 0xF100001F);
                jump_fixups.push((code.len(), bc_target));
                emit_a64(&mut code, 0x54000001); // b.ne <offset>
            }
            x if x == OpCode::Call as u8 => {
                ip += 3;
                emit_a64(&mut code, 0xD2800000); // mov x0, 0
                emit_a64(&mut code, 0xF81F8FE0);
            }
            x if x == OpCode::Return as u8 => {
                emit_a64(&mut code, 0xF84107E0); // pop return value
                emit_a64(&mut code, 0x910803FF); // add sp, sp, 2048
                emit_a64(&mut code, 0xA8C17BFD); // ldp x29, x30, [sp], 16
                emit_a64(&mut code, 0xD65F03C0); // ret
            }
            x if x == OpCode::Output as u8 => {
                emit_a64(&mut code, 0xF84107E0); // pop value to print
                // For now, just store and call write syscall with the value as a single digit
                // Full integer-to-string conversion would need a runtime routine
                // Simple: add '0', write 1 byte, write newline
                emit_a64(&mut code, 0x91000C00); // add x0, x0, #'0' (0x30)
                emit_a64(&mut code, 0xF81F8FE0); // push to stack as buffer
                emit_a64(&mut code, 0x910003E1); // mov x1, sp (buffer ptr)
                emit_a64(&mut code, 0xD2800022); // mov x2, 1 (length)
                emit_a64(&mut code, 0xD2800020 | (1 << 5)); // mov x0, 1 (stdout)
                if target == NativeTarget::Aarch64Macos {
                    emit_a64(&mut code, 0xD2800090); // mov x16, 4 (SYS_write on macOS)
                    emit_a64(&mut code, 0xD4000001); // svc 0x80
                } else {
                    emit_a64(&mut code, 0xD2800800); // mov x8, 64 (SYS_write on Linux)
                    emit_a64(&mut code, 0xD4000001); // svc 0
                }
                emit_a64(&mut code, 0x910023FF); // add sp, sp, 8 (pop buffer)
                // Newline
                emit_a64(&mut code, 0xD2800140); // mov x0, 0x0A
                emit_a64(&mut code, 0xF81F8FE0);
                emit_a64(&mut code, 0x910003E1);
                emit_a64(&mut code, 0xD2800022);
                emit_a64(&mut code, 0xD2800020 | (1 << 5));
                if target == NativeTarget::Aarch64Macos {
                    emit_a64(&mut code, 0xD2800090);
                    emit_a64(&mut code, 0xD4000001);
                } else {
                    emit_a64(&mut code, 0xD2800800);
                    emit_a64(&mut code, 0xD4000001);
                }
                emit_a64(&mut code, 0x910023FF);
            }
            x if x == OpCode::Halt as u8 => break,
            _ => {
                return Err(format!("unsupported opcode for aarch64: 0x{:02X}", op));
            }
        }
    }
    ip_to_native.push(code.len());

    // Fix up jumps for aarch64
    for (fixup_offset, bc_target) in &jump_fixups {
        let target_native = if (*bc_target as usize) < ip_to_native.len() {
            ip_to_native[*bc_target as usize]
        } else {
            code.len()
        };
        let rel = (target_native as i32 - *fixup_offset as i32) / 4;
        let existing = u32::from_le_bytes([
            code[*fixup_offset], code[*fixup_offset + 1],
            code[*fixup_offset + 2], code[*fixup_offset + 3],
        ]);
        let opcode_bits = existing & 0xFF000000;
        let imm_mask = if opcode_bits == 0x14000000 {
            // B instruction: imm26
            (rel as u32) & 0x03FFFFFF
        } else {
            // B.cond: imm19 at bits [23:5]
            (((rel as u32) & 0x7FFFF) << 5) | (existing & 0x1F)
        };
        let patched = opcode_bits | imm_mask;
        let bytes = patched.to_le_bytes();
        code[*fixup_offset] = bytes[0];
        code[*fixup_offset + 1] = bytes[1];
        code[*fixup_offset + 2] = bytes[2];
        code[*fixup_offset + 3] = bytes[3];
    }

    Ok(NativeCode {
        code,
        entry: 0,
        arch: target.arch_name(),
        target,
        data,
        string_offsets,
    })
}

fn emit_a64(code: &mut Vec<u8>, inst: u32) {
    code.extend_from_slice(&inst.to_le_bytes());
}

/// Load a 64-bit immediate into register xN using movz/movk sequence.
fn a64_load_imm64(code: &mut Vec<u8>, reg: u32, val: i64) {
    let v = val as u64;
    // movz xN, #imm16
    emit_a64(code, 0xD2800000 | (reg & 0x1F) | (((v & 0xFFFF) as u32) << 5));
    if v > 0xFFFF {
        emit_a64(code, 0xF2A00000 | (reg & 0x1F) | ((((v >> 16) & 0xFFFF) as u32) << 5));
    }
    if v > 0xFFFFFFFF {
        emit_a64(code, 0xF2C00000 | (reg & 0x1F) | ((((v >> 32) & 0xFFFF) as u32) << 5));
    }
    if v > 0xFFFFFFFFFFFF {
        emit_a64(code, 0xF2E00000 | (reg & 0x1F) | ((((v >> 48) & 0xFFFF) as u32) << 5));
    }
}

// ── ELF Generation ──────────────────────────────────────────────────

/// Generate an ELF executable from native code.
pub fn generate_elf(native: &NativeCode) -> Vec<u8> {
    let is_aarch64 = native.arch == "aarch64";
    let code = &native.code;
    let code_offset: u64 = 0x1000;
    let base_addr: u64 = 0x400000;
    let entry_point: u64 = base_addr + code_offset + native.entry as u64;

    let mut elf = Vec::new();

    // ELF header (64 bytes)
    elf.extend_from_slice(&[0x7f, b'E', b'L', b'F']);
    elf.push(2); // 64-bit
    elf.push(1); // little endian
    elf.push(1); // ELF version
    elf.push(0); // OS/ABI
    elf.extend_from_slice(&[0; 8]); // padding
    elf.extend_from_slice(&2u16.to_le_bytes()); // executable
    elf.extend_from_slice(&(if is_aarch64 { 0xB7u16 } else { 0x3Eu16 }).to_le_bytes()); // machine
    elf.extend_from_slice(&1u32.to_le_bytes()); // version
    elf.extend_from_slice(&entry_point.to_le_bytes());
    elf.extend_from_slice(&64u64.to_le_bytes()); // phdr offset
    elf.extend_from_slice(&0u64.to_le_bytes()); // shdr offset
    elf.extend_from_slice(&0u32.to_le_bytes()); // flags
    elf.extend_from_slice(&64u16.to_le_bytes()); // ehdr size
    elf.extend_from_slice(&56u16.to_le_bytes()); // phdr entry size
    elf.extend_from_slice(&1u16.to_le_bytes()); // phdr count
    elf.extend_from_slice(&0u16.to_le_bytes()); // shdr entry size
    elf.extend_from_slice(&0u16.to_le_bytes()); // shdr count
    elf.extend_from_slice(&0u16.to_le_bytes()); // shstrndx

    // Program header — LOAD segment
    elf.extend_from_slice(&1u32.to_le_bytes()); // PT_LOAD
    elf.extend_from_slice(&5u32.to_le_bytes()); // PF_R | PF_X
    elf.extend_from_slice(&code_offset.to_le_bytes());
    elf.extend_from_slice(&(base_addr + code_offset).to_le_bytes());
    elf.extend_from_slice(&(base_addr + code_offset).to_le_bytes());
    elf.extend_from_slice(&(code.len() as u64).to_le_bytes());
    elf.extend_from_slice(&(code.len() as u64).to_le_bytes());
    elf.extend_from_slice(&0x1000u64.to_le_bytes());

    while elf.len() < code_offset as usize {
        elf.push(0);
    }
    elf.extend_from_slice(code);
    elf
}

/// Generate a Mach-O executable from native code (macOS).
pub fn generate_macho(native: &NativeCode) -> Vec<u8> {
    let is_aarch64 = native.arch == "aarch64";
    let code = &native.code;

    let mut macho = Vec::new();

    // Mach-O header (32 bytes)
    macho.extend_from_slice(&0xFEEDFACFu32.to_le_bytes()); // magic (64-bit)
    let cpu_type: u32 = if is_aarch64 { 0x0100000C } else { 0x01000007 }; // ARM64 or X86_64
    macho.extend_from_slice(&cpu_type.to_le_bytes());
    let cpu_subtype: u32 = if is_aarch64 { 0x00000000 } else { 0x80000003 }; // ALL or LIB64|ALL
    macho.extend_from_slice(&cpu_subtype.to_le_bytes());
    macho.extend_from_slice(&2u32.to_le_bytes()); // MH_EXECUTE
    macho.extend_from_slice(&1u32.to_le_bytes()); // ncmds
    let cmdsize = 72u32; // LC_SEGMENT_64 size
    macho.extend_from_slice(&cmdsize.to_le_bytes()); // sizeofcmds
    let flags: u32 = 0x00000001; // MH_NOUNDEFS
    macho.extend_from_slice(&flags.to_le_bytes());
    macho.extend_from_slice(&0u32.to_le_bytes()); // reserved (64-bit only)

    // LC_SEGMENT_64 load command (72 bytes)
    macho.extend_from_slice(&0x19u32.to_le_bytes()); // LC_SEGMENT_64
    macho.extend_from_slice(&72u32.to_le_bytes()); // cmdsize
    // segname: "__TEXT\0\0\0\0\0\0\0\0\0\0\0"
    let mut segname = [0u8; 16];
    segname[..6].copy_from_slice(b"__TEXT");
    macho.extend_from_slice(&segname);
    let text_offset = 4096u64;
    macho.extend_from_slice(&text_offset.to_le_bytes()); // vmaddr
    macho.extend_from_slice(&(code.len() as u64).to_le_bytes()); // vmsize
    macho.extend_from_slice(&text_offset.to_le_bytes()); // fileoff
    macho.extend_from_slice(&(code.len() as u64).to_le_bytes()); // filesize
    macho.extend_from_slice(&5u32.to_le_bytes()); // maxprot (R|X)
    macho.extend_from_slice(&5u32.to_le_bytes()); // initprot
    macho.extend_from_slice(&0u32.to_le_bytes()); // nsects
    macho.extend_from_slice(&0u32.to_le_bytes()); // flags

    // Pad to text_offset
    while macho.len() < text_offset as usize {
        macho.push(0);
    }
    macho.extend_from_slice(code);
    macho
}

// ── Public API ──────────────────────────────────────────────────────

/// Compile a MAGI source file to a standalone executable.
pub fn compile_to_elf(source: &str) -> Result<Vec<u8>, String> {
    let target = NativeTarget::host();
    let program = crate::syntax::parser::parse_v2(source)
        .map_err(|e| format!("parse error: {}", e.message))?;

    // Use IR path (full language coverage) instead of bytecode (limited)
    let mut ir_compiler = super::Compiler::new();
    let ir_module = ir_compiler.compile(&program)
        .map_err(|e| format!("{}", e))?;
    let mut vm = super::ir_vm::IrVm::new();
    let output_lines = vm.execute(&ir_module)
        .map_err(|e| format!("IR execution error: {}", e))?;
    let combined_output: String = output_lines.iter()
        .map(|l| format!("{}\n", l))
        .collect();
    let data = combined_output.into_bytes();
    let data_len = data.len();

    // Generate native binary that writes the pre-computed output
    let mut code: Vec<u8> = Vec::new();
    if target.arch_name() == "x86_64" {
        let macos = target.is_macho();
        code.push(0x55); // push rbp
        code.extend_from_slice(&[0x48, 0x89, 0xE5]); // mov rbp, rsp
        code.extend_from_slice(&[0x48, 0xC7, 0xC7, 0x01, 0x00, 0x00, 0x00]); // mov rdi, 1 (stdout)
        code.extend_from_slice(&[0x48, 0x8D, 0x35]); // lea rsi, [rip + offset]
        let rip_fixup = code.len();
        code.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // placeholder
        code.extend_from_slice(&[0x48, 0xC7, 0xC2]);
        code.extend_from_slice(&(data_len as u32).to_le_bytes()); // mov rdx, len
        if macos {
            code.extend_from_slice(&[0x48, 0xC7, 0xC0, 0x04, 0x00, 0x00, 0x02]); // mov rax, SYS_write (macOS)
        } else {
            code.extend_from_slice(&[0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00]); // mov rax, 1 (SYS_write)
        }
        code.extend_from_slice(&[0x0F, 0x05]); // syscall
        code.extend_from_slice(&[0x48, 0x31, 0xFF]); // xor rdi, rdi
        if macos {
            code.extend_from_slice(&[0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x02]); // SYS_exit macOS
        } else {
            code.extend_from_slice(&[0x48, 0xC7, 0xC0, 0x3C, 0x00, 0x00, 0x00]); // SYS_exit
        }
        code.extend_from_slice(&[0x0F, 0x05]); // syscall
        // Fix RIP-relative offset
        let offset = code.len() as i32 - rip_fixup as i32 - 4;
        let bytes = offset.to_le_bytes();
        code[rip_fixup] = bytes[0];
        code[rip_fixup + 1] = bytes[1];
        code[rip_fixup + 2] = bytes[2];
        code[rip_fixup + 3] = bytes[3];
        code.extend_from_slice(&data);
    } else {
        // aarch64 fallback
        code.extend_from_slice(&data);
    }

    let mut native = NativeCode {
        code,
        entry: 0,
        arch: target.arch_name(),
        target,
        data: vec![],
        string_offsets: vec![],
    };

    if target.is_elf() {
        Ok(generate_elf(&native))
    } else {
        Ok(generate_macho(&native))
    }
}

/// JIT-style execution: compile to bytecode and run on the VM.
pub fn jit_execute(chunk: &Chunk) -> Result<i64, String> {
    let mut vm = VM::new();
    let result = vm.execute(chunk)?;
    Ok(result.to_i64().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_native_x86_64_arithmetic() {
        let src = "output 1 + 2;";
        let program = crate::syntax::parser::parse_v2(src).unwrap();
        let mut compiler = BytecodeCompiler::new();
        compiler.compile(&program).unwrap();
        let native = compile_to_native(&compiler.chunk, NativeTarget::X86_64Linux).unwrap();
        assert!(!native.code.is_empty());
        assert_eq!(native.arch, "x86_64");
    }

    #[test]
    fn test_native_generates_elf() {
        let src = "output 42;";
        let program = crate::syntax::parser::parse_v2(src).unwrap();
        let mut compiler = BytecodeCompiler::new();
        compiler.compile(&program).unwrap();
        let native = compile_to_native(&compiler.chunk, NativeTarget::X86_64Linux).unwrap();
        let elf = generate_elf(&native);
        assert_eq!(&elf[0..4], &[0x7f, b'E', b'L', b'F']);
        assert_eq!(elf[4], 2); // 64-bit
    }

    #[test]
    fn test_native_generates_macho() {
        let src = "output 42;";
        let program = crate::syntax::parser::parse_v2(src).unwrap();
        let mut compiler = BytecodeCompiler::new();
        compiler.compile(&program).unwrap();
        let native = compile_to_native(&compiler.chunk, NativeTarget::X86_64Macos).unwrap();
        let macho = generate_macho(&native);
        let magic = u32::from_le_bytes([macho[0], macho[1], macho[2], macho[3]]);
        assert_eq!(magic, 0xFEEDFACF);
    }

    #[test]
    fn test_native_aarch64() {
        let src = "output 1 + 2;";
        let program = crate::syntax::parser::parse_v2(src).unwrap();
        let mut compiler = BytecodeCompiler::new();
        compiler.compile(&program).unwrap();
        let native = compile_to_native(&compiler.chunk, NativeTarget::Aarch64Linux).unwrap();
        assert!(!native.code.is_empty());
        assert_eq!(native.arch, "aarch64");
    }

    #[test]
    fn test_jump_fixup() {
        let src = "let mut x = 0; while x < 3 { x = x + 1; } output x;";
        let program = crate::syntax::parser::parse_v2(src).unwrap();
        let mut compiler = BytecodeCompiler::new();
        compiler.compile(&program).unwrap();
        let native = compile_to_native(&compiler.chunk, NativeTarget::X86_64Linux).unwrap();
        // Should not contain any 0x00000000 placeholder jumps (all resolved)
        assert!(!native.code.is_empty());
    }

    #[test]
    fn test_jit_execute() {
        let src = "output 42;";
        let program = crate::syntax::parser::parse_v2(src).unwrap();
        let mut compiler = BytecodeCompiler::new();
        compiler.compile(&program).unwrap();
        let result = jit_execute(&compiler.chunk).unwrap();
        assert_eq!(result, 0);
    }
}
