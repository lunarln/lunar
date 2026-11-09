//! Target initialization shims.
//!
//! inkwell references llvm-sys's C wrapper functions (`LLVM_InitializeAll*`,
//! `LLVM_InitializeNative*`), which are only built when `llvm-config` is
//! available. We define them here in Rust on top of the per-target functions
//! exported by the LLVM shared library, for the targets Lunar supports.

#![allow(non_snake_case)]

unsafe extern "C" {
    fn LLVMInitializeX86TargetInfo();
    fn LLVMInitializeX86Target();
    fn LLVMInitializeX86TargetMC();
    fn LLVMInitializeX86AsmPrinter();
    fn LLVMInitializeX86AsmParser();
    fn LLVMInitializeX86Disassembler();
    fn LLVMInitializeAArch64TargetInfo();
    fn LLVMInitializeAArch64Target();
    fn LLVMInitializeAArch64TargetMC();
    fn LLVMInitializeAArch64AsmPrinter();
    fn LLVMInitializeAArch64AsmParser();
    fn LLVMInitializeAArch64Disassembler();
}

#[unsafe(no_mangle)]
extern "C" fn LLVM_InitializeAllTargetInfos() {
    unsafe {
        LLVMInitializeX86TargetInfo();
        LLVMInitializeAArch64TargetInfo();
    }
}

#[unsafe(no_mangle)]
extern "C" fn LLVM_InitializeAllTargets() {
    unsafe {
        LLVMInitializeX86Target();
        LLVMInitializeAArch64Target();
    }
}

#[unsafe(no_mangle)]
extern "C" fn LLVM_InitializeAllTargetMCs() {
    unsafe {
        LLVMInitializeX86TargetMC();
        LLVMInitializeAArch64TargetMC();
    }
}

#[unsafe(no_mangle)]
extern "C" fn LLVM_InitializeAllAsmPrinters() {
    unsafe {
        LLVMInitializeX86AsmPrinter();
        LLVMInitializeAArch64AsmPrinter();
    }
}

#[unsafe(no_mangle)]
extern "C" fn LLVM_InitializeAllAsmParsers() {
    unsafe {
        LLVMInitializeX86AsmParser();
        LLVMInitializeAArch64AsmParser();
    }
}

#[unsafe(no_mangle)]
extern "C" fn LLVM_InitializeAllDisassemblers() {
    unsafe {
        LLVMInitializeX86Disassembler();
        LLVMInitializeAArch64Disassembler();
    }
}

#[unsafe(no_mangle)]
extern "C" fn LLVM_InitializeNativeTarget() -> i32 {
    unsafe {
        if cfg!(target_arch = "aarch64") {
            LLVMInitializeAArch64TargetInfo();
            LLVMInitializeAArch64Target();
            LLVMInitializeAArch64TargetMC();
        } else {
            LLVMInitializeX86TargetInfo();
            LLVMInitializeX86Target();
            LLVMInitializeX86TargetMC();
        }
    }
    0
}

#[unsafe(no_mangle)]
extern "C" fn LLVM_InitializeNativeAsmPrinter() -> i32 {
    unsafe {
        if cfg!(target_arch = "aarch64") {
            LLVMInitializeAArch64AsmPrinter();
        } else {
            LLVMInitializeX86AsmPrinter();
        }
    }
    0
}

#[unsafe(no_mangle)]
extern "C" fn LLVM_InitializeNativeAsmParser() -> i32 {
    unsafe {
        if cfg!(target_arch = "aarch64") {
            LLVMInitializeAArch64AsmParser();
        } else {
            LLVMInitializeX86AsmParser();
        }
    }
    0
}

#[unsafe(no_mangle)]
extern "C" fn LLVM_InitializeNativeDisassembler() -> i32 {
    unsafe {
        if cfg!(target_arch = "aarch64") {
            LLVMInitializeAArch64Disassembler();
        } else {
            LLVMInitializeX86Disassembler();
        }
    }
    0
}

/// Initialize the native target for code generation.
pub fn native() {
    LLVM_InitializeNativeTarget();
    LLVM_InitializeNativeAsmPrinter();
    LLVM_InitializeNativeAsmParser();
}
