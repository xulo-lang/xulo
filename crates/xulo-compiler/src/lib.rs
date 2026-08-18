// `xulo-codegen` is deprecated and about to be folded into this crate; the
// pipeline below still relies on it until then.
#![allow(deprecated)]

pub mod module;

use std::path::Path;

use xulo_core::error::XuloError;

/// Full pipeline: tokenize -> parse -> semantic check -> generate JavaScript.
pub fn compile(source: &str, _file: &Path) -> Result<String, XuloError> {
    let tokens = xulo_lexer::tokenize(source)?;
    let mut ast = xulo_parser::parse_program(&tokens)?;
    let result = xulo_semantic::analyze_with(&ast, &[], &[], &[])?;
    xulo_semantic::apply_trait_dispatch(&mut ast, &result.trait_dispatch);
    xulo_semantic::apply_list_concat(&mut ast, &result.list_concat);
    xulo_codegen::generate(&ast)
}
