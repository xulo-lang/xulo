//! The AOT backend sees the expanded program and omits `#[macro]`
//! implementation bodies, so a macro-using program still lowers to IR.

use std::path::Path;

#[test]
fn macro_program_compiles_to_ir() {
    let src = r#"
#[macro(kind = "function", name = "answer")]
fn answer_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  return Result::Ok(ast::FunctionOutput(expr: (ast::quote! { 42 })[0]))
}
fn main() { print(answer!()) }
"#;
    xulo_compiler::compile_to_ir(src, Path::new("t.xulo")).unwrap();
}
