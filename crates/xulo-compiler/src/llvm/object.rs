//! LLVM 目标文件输出。
//!
//! 使用宿主 target triple 生成非 PIC 目标文件（与 `cc -no-pie` 链接方式一致）；
//! `XULO_DUMP_LLVM=1` 时把 `.ll` 文本写到目标文件旁边，供语言开发者调试。

use std::path::Path;

use inkwell::OptimizationLevel;
use inkwell::module::Module;
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};

use xulo_core::error::{ErrorKind, XuloError};

fn codegen_err(msg: impl std::fmt::Display) -> XuloError {
    XuloError::new(ErrorKind::Codegen, msg.to_string())
}

pub fn write_object(module: &Module, obj_path: &Path, optimize: bool) -> Result<(), XuloError> {
    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| codegen_err(format!("failed to initialize LLVM native target: {e}")))?;

    let triple = TargetMachine::get_default_triple();
    module.set_triple(&triple);

    let target = Target::from_triple(&triple)
        .map_err(|e| codegen_err(format!("failed to get LLVM target: {e}")))?;
    let level = if optimize {
        OptimizationLevel::Aggressive
    } else {
        OptimizationLevel::None
    };
    let machine = target
        .create_target_machine(
            &triple,
            "",
            "",
            level,
            RelocMode::Static,
            CodeModel::Default,
        )
        .ok_or_else(|| codegen_err("failed to create LLVM target machine"))?;

    module.set_data_layout(&machine.get_target_data().get_data_layout());

    // `create_target_machine` 的优化等级只影响指令选择，不会运行 IR 优化管线；
    // 若不显式 run_passes，`alloca`/`load`/`store` 不会被 mem2reg 提升成 SSA，
    // 循环和递归的性能会明显落后于 Cranelift。release profile 跑 default<O3>。
    if optimize {
        let options = PassBuilderOptions::create();
        options.set_verify_each(false);
        module
            .run_passes("default<O3>", &machine, options)
            .map_err(|e| codegen_err(format!("LLVM optimization pipeline failed: {e}")))?;
    }

    if std::env::var_os("XULO_DUMP_LLVM").is_some() {
        let dump_path = obj_path.with_extension("ll");
        std::fs::write(&dump_path, module.print_to_string().to_string())
            .map_err(|e| codegen_err(format!("failed to write {}: {e}", dump_path.display())))?;
    }

    machine
        .write_to_file(module, FileType::Object, obj_path)
        .map_err(|e| codegen_err(format!("failed to write object file: {e}")))
}
