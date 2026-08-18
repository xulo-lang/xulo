//! # Deprecated
//!
//! This crate is **deprecated** and scheduled for removal: its JavaScript
//! generation is orchestrated by `xulo-compiler`, which should take it over.
//! New code must not depend on this crate; existing callers rely on it only
//! through `xulo-compiler`'s pipeline (`compile`, module loading/bundling).

#![deprecated(
    since = "0.1.0",
    note = "JavaScript generation is being folded into `xulo-compiler`; this crate will be removed in a future release"
)]
// `generate` below is the remaining live API and still uses the (deprecated)
// `javascript` module; suppress the crate's own deprecation warnings.
#![allow(deprecated)]

#[deprecated(
    since = "0.1.0",
    note = "will be folded into `xulo-compiler` and this crate removed"
)]
pub mod javascript;

use xulo_core::ast::Program;
use xulo_core::error::XuloError;

use self::javascript::Javascript;

/// Generate a JavaScript (ES module) string for a program.
///
/// Deprecated: use `xulo_compiler::compile` (the full pipeline) instead.
#[deprecated(
    since = "0.1.0",
    note = "use `xulo_compiler::compile` instead; this crate is scheduled for removal"
)]
pub fn generate(program: &Program) -> Result<String, XuloError> {
    let mut codegen = Javascript::new();
    codegen.program(program)?;
    Ok(codegen.finish())
}
