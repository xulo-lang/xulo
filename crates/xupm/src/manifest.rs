//! Reading a project's `xupm.toml` and locating package/workspace roots.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

pub const MANIFEST_FILE: &str = "xupm.toml";
pub const DEFAULT_ENTRY: &str = "main.xulo";

/// The fields `xupm` needs from a project manifest. A manifest may describe a
/// package (`[package]`), a workspace (`[workspace]`), or both (a package that
/// also roots a workspace, like Cargo's `Cargo.toml`).
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub edition: String,
    pub entry: PathBuf,
    /// `[workspace].members`, or `None` when this is not a workspace root.
    pub workspace_members: Option<Vec<String>>,
    /// Whether the manifest has a `[package]` table.
    pub has_package: bool,
}

/// A discovered project: the directory holding `xupm.toml` and its manifest.
pub struct Project {
    pub root: PathBuf,
    pub manifest: Manifest,
}

impl Project {
    pub fn entry(&self) -> PathBuf {
        self.root.join(&self.manifest.entry)
    }

    /// A workspace whose root is not itself a package. Its default members are
    /// every workspace package; a root that is also a package defaults to
    /// itself (Cargo's rule).
    pub fn is_virtual_workspace(&self) -> bool {
        self.manifest.workspace_members.is_some() && !self.manifest.has_package
    }

    /// Every package directory under this project: the root itself when it is
    /// a package, plus the expanded workspace `members`.
    pub fn package_dirs(&self) -> Result<Vec<PathBuf>, String> {
        let Some(members) = &self.manifest.workspace_members else {
            if self.manifest.has_package {
                return Ok(vec![self.root.clone()]);
            }
            return Ok(Vec::new());
        };
        let mut dirs = BTreeSet::new();
        if self.manifest.has_package {
            dirs.insert(self.root.clone());
        }
        for pattern in members {
            for dir in expand_member(&self.root, pattern)? {
                dirs.insert(dir);
            }
        }
        Ok(dirs.into_iter().collect())
    }
}

/// Walk up from `start` (a file or directory) to the nearest directory that
/// contains `xupm.toml`.
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

/// Read the manifest in `dir` as a project root.
pub fn load(dir: &Path) -> Result<Project, String> {
    let manifest = read_manifest(&dir.join(MANIFEST_FILE), dir)?;
    Ok(Project {
        root: dir.to_path_buf(),
        manifest,
    })
}

fn read_manifest(path: &Path, root: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let table: toml::Table = text
        .parse()
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    let package = table.get("package").and_then(toml::Value::as_table);
    let workspace = table.get("workspace").and_then(toml::Value::as_table);
    if package.is_none() && workspace.is_none() {
        return Err(format!(
            "{} has neither a [package] nor a [workspace] table",
            path.display()
        ));
    }
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
    let workspace_members = workspace.map(|workspace| {
        workspace
            .get("members")
            .and_then(toml::Value::as_array)
            .map(|members| {
                members
                    .iter()
                    .filter_map(toml::Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    });
    Ok(Manifest {
        name: field("name").unwrap_or(default_name),
        version: field("version").unwrap_or_else(|| "0.1.0".to_string()),
        edition: field("edition").unwrap_or_else(|| "2024".to_string()),
        entry: PathBuf::from(field("entry").unwrap_or_else(|| DEFAULT_ENTRY.to_string())),
        workspace_members,
        has_package: package.is_some(),
    })
}

/// Expand one `[workspace].members` entry. A `*` stands for one path segment;
/// every other segment is taken literally. Entries are relative to the
/// workspace root.
fn expand_member(root: &Path, pattern: &str) -> Result<Vec<PathBuf>, String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    let mut current = vec![root.to_path_buf()];
    for component in Path::new(pattern).components() {
        match component {
            Component::Normal(segment) if segment == OsStr::new("*") => {
                let mut next = Vec::new();
                for dir in &current {
                    next.extend(subdirectories(dir)?);
                }
                current = next;
            }
            Component::Normal(segment) => {
                current = current.into_iter().map(|dir| dir.join(segment)).collect();
            }
            Component::CurDir => {}
            Component::ParentDir => {
                current = current.into_iter().map(|dir| dir.join("..")).collect();
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "workspace member `{pattern}` must be relative to the workspace root"
                ));
            }
        }
    }
    let has_glob = pattern.split(['/', '\\']).any(|segment| segment == "*");
    let mut members = Vec::new();
    for dir in current {
        if dir.join(MANIFEST_FILE).is_file() {
            members.push(dir);
        } else if !has_glob {
            return Err(format!(
                "workspace member `{pattern}` has no {MANIFEST_FILE}"
            ));
        }
    }
    if members.is_empty() {
        return Err(format!("no workspace member matches `{pattern}`"));
    }
    members.sort();
    Ok(members)
}

fn subdirectories(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read workspace member path {}: {e}", dir.display()))?;
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.path())
        .collect();
    dirs.sort();
    Ok(dirs)
}
