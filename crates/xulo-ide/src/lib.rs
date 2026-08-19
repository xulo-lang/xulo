//! Editor/toolchain analysis layer for Xulo (rust-analyzer's role for VS
//! Code): parses, checks, and exposes *queryable* analysis results (name
//! resolution, expression types, document outline, diagnostics) without
//! entering the compile pipeline.
//!
//! The crate does not feed `xulo-compiler` / `xulo-runtime`; it consumes the
//! out-of-band recording done by `xulo-semantic`'s checker
//! (`AnalysisResult.expr_types` / `.resolutions`) plus its own module graph.

pub mod analysis;
pub mod diagnostics;
pub mod format;
pub mod line_index;
pub mod object;
pub mod queries;
pub mod semantic_tokens;
pub mod workspace;

pub use analysis::{Analysis, analyze_source};
