//! The native standard library of the Xulo language.
//!
//! This crate owns the run-time semantics of the built-in namespaces — `Math`,
//! `Time`, `Fs`, and `Io` (spec [`builtins/intrinsic-functions.md`](../../../spec/builtins/intrinsic-functions.md))
//! — plus the [`regex`] module behind the language's `RegExp` type, as pure
//! Rust functions, and the `extern "C"` entry points ([`ffi`]) that compiled
//! (JIT/AOT) code calls. The interpreter (`xulo-runtime`) and the compiler
//! backends (`xulo-compiler`) both go through this crate, so the interpreted
//! and compiled paths share one implementation and cannot drift apart.
//!
//! The crate deliberately carries no `Value`/`RunError`: argument-count and
//! argument-type guards stay in the interpreter, which reports violations as
//! `RunError`s with source spans. Everything here assumes a well-typed call
//! (the semantic pass enforces the registered signature).

pub mod ffi;
pub mod format;
pub mod fs;
pub mod io;
pub mod math;
pub mod regex;
pub mod time;

pub use format::format_number;
