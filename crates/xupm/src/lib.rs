//! The Xulo package manager: project manifests, dependency resolution, a
//! lockfile, and package fetching. The compiler stays network-free; `xupm`
//! turns `xupm.toml` into a resolved dependency map and hands it over.

// Distribution artifacts must never silently ship the Cranelift fallback.
#[cfg(all(feature = "distribution", not(feature = "llvm")))]
compile_error!("`distribution` builds must enable the `llvm` feature");

pub mod cli;
pub mod manifest;
