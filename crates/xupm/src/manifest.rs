//! Reading a project's `xulo.toml` and locating the package root.

use std::path::{Path, PathBuf};

pub const MANIFEST_FILE: &str = "xulo.toml";
pub const DEFAULT_ENTRY: &str = "main.xulo";

/// The fields `xupm` needs from a project manifest. The compiler owns the
/// full dependency mapping; this is the package metadata used to find the
/// entry module and name build artifacts.
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub edition: String,
    pub entry: PathBuf,
}

/// A discovered project: the directory holding `xulo.toml` and its manifest.
pub struct Project {
    pub root: PathBuf,
    pub manifest: Manifest,
}

impl Project {
    pub fn entry(&self) -> PathBuf {
        self.root.join(&self.manifest.entry)
    }
}

/// Walk up from `start` (a file or directory) to the nearest directory that
/// contains `xulo.toml`.
pub fn discover(start: &Path) -> Result<Option<Project>, String> {
    let mut dir = if start.is_dir() {
        start.to_path_buf()
    } else {
        match start.parent() {
            Some(parent) => parent.to_path_buf(),
            None => return Ok(None),
        }
    };
    loop {
        let manifest_path = dir.join(MANIFEST_FILE);
        if manifest_path.is_file() {
            let manifest = read_manifest(&manifest_path, &dir)?;
            return Ok(Some(Project {
                root: dir,
                manifest,
            }));
        }
        if !dir.pop() {
            return Ok(None);
        }
    }
}

fn read_manifest(path: &Path, root: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let table: toml::Table = text
        .parse()
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    let package = table.get("package").and_then(toml::Value::as_table);
    let default_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("package")
        .to_string();
    let field = |key: &str| {
        package
            .and_then(|p| p.get(key))
            .and_then(toml::Value::as_str)
            .map(str::to_string)
    };
    Ok(Manifest {
        name: field("name").unwrap_or(default_name),
        version: field("version").unwrap_or_else(|| "0.1.0".to_string()),
        edition: field("edition").unwrap_or_else(|| "2024".to_string()),
        entry: PathBuf::from(field("entry").unwrap_or_else(|| DEFAULT_ENTRY.to_string())),
    })
}
