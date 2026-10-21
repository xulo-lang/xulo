pub mod aot;
pub mod backend;
pub mod codegen;
pub mod irgen;
/// JavaScript generation has been retired: programs run on the native
/// interpreter (`xulo_runtime`). This crate handles the front end
/// (tokenize -> parse -> semantic check) and multi-file module loading for the
/// native runner.
pub mod module;

use std::path::Path;

use xulo_core::error::XuloError;
use xulo_core::ir::IrModule;

/// Front-end pipeline for a single file: tokenize -> parse -> semantic check,
/// returning any non-fatal warnings raised during analysis. JS generation has
/// been retired; executing programs is the native interpreter's job.
pub fn compile(source: &str, file: &Path) -> Result<Vec<XuloError>, XuloError> {
    let program =
        xulo_parser::parse_program(source).map_err(|e| e.with_file(file.to_path_buf()))?;
    let result = xulo_semantic::analyze_with(&program, &[], &[], &[])?;
    Ok(result.warnings)
}

/// 编译源代码到 IR
pub fn compile_to_ir(source: &str, file: &Path) -> Result<IrModule, XuloError> {
    let program =
        xulo_parser::parse_program(source).map_err(|e| e.with_file(file.to_path_buf()))?;
    let result = xulo_semantic::analyze_with(&program, &[], &[], &[])?;

    // 应用语义分析的修改
    let mut ast = program;
    xulo_semantic::apply_trait_dispatch(&mut ast, &result.trait_dispatch);
    xulo_semantic::apply_list_concat(&mut ast, &result.list_concat);
    // 裸名 std 内建调用（`use std::math::*` 后的 `floor(…)`）改写为限定形式，
    // AOT lowering 只认 `std::math::…` 键。
    xulo_semantic::apply_std_calls(&mut ast, &result.std_calls);

    // 生成 IR
    irgen::generate_ir(&ast)
}

/// 编译整棵模块树为一个 IR 模块：入口模块（id `""`）的顶层名字保留原样，
/// 其余模块按 `模块 id::名字` 加前缀；`use`/`mod` 绑定在 IR 生成期静态解析。
/// 调用前须先 [`module::load`] + [`module::analyze`]，并应用
/// [`module::apply_trait_dispatch`] / [`module::apply_std_calls`] 的标注。
pub fn compile_modules_to_ir(loaded: &module::LoadedModules) -> Result<IrModule, XuloError> {
    irgen::IrGenerator::new().generate_modules(loaded)
}
