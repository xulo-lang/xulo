//! Native tree-walking interpreter for the core Xulo language.
//!
//! Runs an already-parsed [`Program`] directly in Rust — no JavaScript, no
//! Node.js. The core language covers literals, variables, functions, closures,
//! recursion, control flow, match, enums, lists/objects, error handling,
//! `async`/`await` (a cooperative scheduler built on stackful coroutines), and
//! local module execution. UI components, external `import`s, `@State`
//! bindings, and `$` bindings are rejected with a clear error.

pub mod env;
pub mod interpreter;
pub mod value;
pub mod runtime;
