pub mod module;

use std::path::Path;

use xulo_core::error::XuloError;

/// Full pipeline: tokenize -> parse -> semantic check -> generate JavaScript.
pub fn compile(source: &str, _file: &Path) -> Result<String, XuloError> {
    let tokens = xulo_lexer::tokenize(source)?;
    let ast = xulo_parser::parse_program(&tokens)?;
    xulo_semantic::analyze(&ast)?;
    xulo_codegen::generate(&ast)
}
