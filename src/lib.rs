pub mod ast;
pub mod cli;
pub mod codegen;
pub mod diagnostics;
pub mod error;
pub mod formatter;
pub mod lexer;
pub mod module;
pub mod parser;
pub mod semantic;

use std::path::Path;

use crate::error::XuloError;

/// Full pipeline: tokenize -> parse -> semantic check -> generate JavaScript.
pub fn compile(source: &str, _file: &Path) -> Result<String, XuloError> {
    let tokens = lexer::tokenize(source)?;
    let ast = parser::parse_program(&tokens)?;
    semantic::analyze(&ast)?;
    codegen::generate(&ast)
}