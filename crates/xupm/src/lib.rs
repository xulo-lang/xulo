//! The Xulo package manager: project manifests, dependency resolution, a
//! lockfile, and package fetching. The compiler stays network-free; `xupm`
//! turns `xulo.toml` into a resolved dependency map and hands it over.

pub mod cli;
pub mod manifest;
