/// JavaScript generation has been retired: programs run on the native
/// interpreter (`xulo_runtime`). This crate handles the front end
/// (tokenize -> parse -> semantic check) and multi-file module loading for the
/// native runner.
pub mod module;

use std::path::Path;

use xulo_core::error::XuloError;

/// Front-end pipeline for a single file: tokenize -> parse -> semantic check,
/// returning any non-fatal warnings raised during analysis. JS generation has
/// been retired; executing programs is the native interpreter's job.
pub fn compile(source: &str, file: &Path) -> Result<Vec<XuloError>, XuloError> {
    let tokens = xulo_lexer::tokenize(source).map_err(|e| e.with_file(file.to_path_buf()))?;
    let program =
        xulo_parser::parse_program(&tokens).map_err(|e| e.with_file(file.to_path_buf()))?;
    let result = xulo_semantic::analyze_with(&program, &[], &[], &[])?;
    Ok(result.warnings)
}
