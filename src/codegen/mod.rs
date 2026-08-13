pub mod javascript;

use crate::ast::Program;
use crate::error::XuloError;

use self::javascript::Javascript;

/// Generate a JavaScript (ES module) string for a program.
pub fn generate(program: &Program) -> Result<String, XuloError> {
    let mut codegen = Javascript::new();
    codegen.program(program)?;
    Ok(codegen.finish())
}