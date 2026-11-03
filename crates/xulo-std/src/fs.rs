//! The `Fs` namespace: the whole-file filesystem operations exposed to Xulo
//! programs. Every fallible operation reports its failure as a message string
//! (`Result<T, Error>` in the language, wrapped by the interpreter with the
//! built-in `Error`); `exists` is a total predicate and never fails. Text is
//! the default — `read` decodes UTF-8 — and the three byte members carry a
//! `List<u8>` so non-text content never crosses a decoder.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The failure message of an operation: which operation, on which path, and
/// the underlying OS error (`cannot read 'a.txt': No such file or directory
/// (os error 2)`).
fn fail(op: &str, path: &str, err: std::io::Error) -> String {
    format!("cannot {op} '{path}': {err}")
}

/// What `stat` reports about one path: its kind, size, modification time, and
/// permission bits (spec `modules/fs.md` Inspecting a file).
pub struct Stat {
    pub kind: String,
    pub size: u64,
    pub modified: u64,
    pub mode: u64,
}

/// `Fs::read(path)`: the entire file as a string; a missing file, a
/// directory, or invalid UTF-8 is a failure.
pub fn read(path: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| fail("read", path, e))
}

/// `Fs::write(path, content)`: create the file or truncate it to `content`;
/// missing parent directories are a failure.
pub fn write(path: &str, content: &str) -> Result<(), String> {
    fs::write(path, content).map_err(|e| fail("write", path, e))
}

/// `Fs::append(path, content)`: grow the file by `content`, creating it when
/// absent; never truncates.
pub fn append(path: &str, content: &str) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| fail("append", path, e))?;
    file.write_all(content.as_bytes())
        .map_err(|e| fail("append", path, e))
}

/// `Fs::exists(path)`: whether a file or directory is there. A path that
/// cannot be probed (permission, invalid syntax) counts as absent, so the
/// predicate stays total.
pub fn exists(path: &str) -> bool {
    Path::new(path).exists()
}

/// `Fs::create_new(path, contents)`: create a new file containing `contents`,
/// failing when anything already exists at `path`. The check and the creation
/// are one host operation, so a concurrent creator cannot slip between them.
pub fn create_new(path: &str, contents: &str) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| fail("create", path, e))?;
    file.write_all(contents.as_bytes())
        .map_err(|e| fail("create", path, e))
}

fn set_modified(path: &str, when: SystemTime) -> Result<(), String> {
    let file = fs::File::open(path).map_err(|e| fail("set times", path, e))?;
    file.set_modified(when)
        .map_err(|e| fail("set times", path, e))
}

fn ms_to_system_time(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn system_time_to_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `Fs::touch(path)`: create an empty file when absent; refresh an existing
/// entry's modification time, keeping every byte it held.
pub fn touch(path: &str) -> Result<(), String> {
    if Path::new(path).exists() {
        return set_modified(path, SystemTime::now());
    }
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| fail("touch", path, e))?;
    Ok(())
}

/// `Fs::set_times(path, modified)`: stamp `path`'s modification time to
/// `modified` milliseconds since the Unix epoch; never creates.
pub fn set_times(path: &str, modified: u64) -> Result<(), String> {
    set_modified(path, ms_to_system_time(modified))
}

/// `Fs::set_mode(path, mode)`: set the entry's permission bits to the low 12
/// bits of a POSIX mode; the domain `0 …= 4095` is checked before the host.
pub fn set_mode(path: &str, mode: u64) -> Result<(), String> {
    if mode > 4095 {
        return Err(format!(
            "cannot set mode '{path}': mode {mode} outside 0 …= 4095"
        ));
    }
    let meta = fs::metadata(path).map_err(|e| fail("set mode", path, e))?;
    let mut perms = meta.permissions();
    perms.set_mode(mode as u32);
    fs::set_permissions(path, perms).map_err(|e| fail("set mode", path, e))
}

/// `Fs::remove(path)`: delete one file; a directory path is a failure.
pub fn remove(path: &str) -> Result<(), String> {
    fs::remove_file(path).map_err(|e| fail("remove", path, e))
}

/// `Fs::mkdir(path)`: create the directory, including missing parents
/// (single-level semantics would leave programs to race on `a/b/c`).
pub fn mkdir(path: &str) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| fail("create directory", path, e))
}

/// `Fs::realpath(path)`: the absolute, link-free path that names what `path`
/// names; a missing component or broken link is a failure.
pub fn realpath(path: &str) -> Result<String, String> {
    fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| fail("resolve", path, e))
}

/// `Fs::stat(path)`: what the path is — kind, size, modification time,
/// permission bits — following symlinks; anything that is not a file or a
/// directory is a failure.
pub fn stat(path: &str) -> Result<Stat, String> {
    let meta = fs::metadata(path).map_err(|e| fail("stat", path, e))?;
    let kind = if meta.is_dir() {
        "directory"
    } else if meta.is_file() {
        "file"
    } else {
        return Err(format!("cannot stat '{path}': not a file or directory"));
    };
    Ok(Stat {
        kind: kind.to_string(),
        size: meta.len(),
        modified: meta.modified().map(system_time_to_ms).unwrap_or(0),
        mode: if cfg!(unix) {
            (meta.permissions().mode() as u64) & 0o7777
        } else {
            0
        },
    })
}

/// `Fs::copy(from, to)`: copy `from`'s bytes to `to`, replacing `to`'s
/// previous content; a copy onto its own source is a failure.
pub fn copy(from: &str, to: &str) -> Result<(), String> {
    if !Path::new(from).exists() {
        return Err(fail(
            "copy",
            from,
            std::io::Error::new(std::io::ErrorKind::NotFound, "no such file or directory"),
        ));
    }
    let same = match (fs::canonicalize(from), fs::canonicalize(to)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if same {
        return Err(format!("cannot copy '{from}' onto its own source"));
    }
    fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| fail("copy", from, e))
}

/// `Fs::rename(from, to)`: move `from` to `to`, replacing `to` when the two
/// agree in kind; a move onto the same path is a failure.
pub fn rename(from: &str, to: &str) -> Result<(), String> {
    let same = match (fs::canonicalize(from), fs::canonicalize(to)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if same {
        return Err(format!("cannot rename '{from}' onto itself"));
    }
    fs::rename(from, to).map_err(|e| fail("rename", from, e))
}

/// `Fs::remove_dir(path)`: delete the directory and everything in it. A
/// symlink at `path` is not a directory — it is removed as a link's target is
/// never deleted here, so the call fails instead.
pub fn remove_dir(path: &str) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| fail("remove directory", path, e))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "cannot remove directory '{path}': not a directory (it is a symbolic link)"
        ));
    }
    fs::remove_dir_all(path).map_err(|e| fail("remove directory", path, e))
}

fn file_name_of(entry: &fs::DirEntry) -> String {
    entry.file_name().to_string_lossy().into_owned()
}

/// `Fs::read_dir(path)`: the bare names of everything in the directory, sorted
/// so the answer is stable across hosts.
pub fn read_dir(path: &str) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = fs::read_dir(path)
        .map_err(|e| fail("read directory", path, e))?
        .map(|entry| entry.map(|e| file_name_of(&e)))
        .collect::<Result<_, _>>()
        .map_err(|e| fail("read directory", path, e))?;
    names.sort();
    Ok(names)
}

fn walk_into(dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let dir_str = dir.to_string_lossy();
    let entries = fs::read_dir(dir).map_err(|e| fail("walk", &dir_str, e))?;
    let mut children: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| fail("walk", &dir_str, e))?;
        let child = dir.join(entry.file_name());
        let meta = fs::symlink_metadata(&child).map_err(|e| fail("walk", &dir_str, e))?;
        // A symlink's path is listed but never descended into, so a link
        // cycle cannot loop and a link out of the tree cannot widen the walk.
        if meta.file_type().is_dir() && !meta.file_type().is_symlink() {
            children.push(child.clone());
        }
        out.push(child.to_string_lossy().into_owned());
    }
    children.sort();
    for child in children {
        walk_into(&child, out)?;
    }
    Ok(())
}

/// `Fs::walk(path)`: the starting path and every path beneath it, the start
/// first and each directory before its contents; a symlink on the way down is
/// listed but never entered.
pub fn walk(path: &str) -> Result<Vec<String>, String> {
    let start = Path::new(path);
    let meta = fs::metadata(start).map_err(|e| fail("walk", path, e))?;
    if !meta.is_dir() {
        return Err(format!("cannot walk '{path}': not a directory"));
    }
    let mut out = vec![path.to_string()];
    walk_into(start, &mut out)?;
    Ok(out)
}

/// `Fs::read_bytes(path)`: the whole file as raw bytes, one element per byte;
/// any content is `Ok`, only path failures are `Err`.
pub fn read_bytes(path: &str) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| fail("read", path, e))
}

/// `Fs::write_bytes(path, bytes)`: create or truncate the file, then write the
/// bytes; no decoding in either direction.
pub fn write_bytes(path: &str, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|e| fail("write", path, e))
}

/// `Fs::append_bytes(path, bytes)`: grow the file by the bytes, creating it
/// when absent; never truncates.
pub fn append_bytes(path: &str, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| fail("append", path, e))?;
    file.write_all(bytes).map_err(|e| fail("append", path, e))
}

fn unique_path(dir: &Path, prefix: &str, is_dir: bool) -> Result<String, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for attempt in 0..u32::MAX {
        let name = format!("{prefix}{}_{nanos}_{attempt}", std::process::id());
        let candidate = dir.join(name);
        let created = if is_dir {
            fs::create_dir(&candidate)
        } else {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
                .map(|_| ())
        };
        match created {
            Ok(()) => return Ok(candidate.to_string_lossy().into_owned()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(fail("create temporary", &candidate.to_string_lossy(), e)),
        }
    }
    Err("cannot create temporary entry: too many collisions".to_string())
}

/// `Fs::temp_file(prefix)`: create a unique empty file in the host's temporary
/// directory and yield its path.
pub fn temp_file(prefix: Option<&str>) -> Result<String, String> {
    temp_file_in(&std::env::temp_dir().to_string_lossy(), prefix)
}

/// `Fs::temp_dir(prefix)`: create a unique empty directory in the host's
/// temporary directory and yield its path.
pub fn temp_dir(prefix: Option<&str>) -> Result<String, String> {
    temp_dir_in(&std::env::temp_dir().to_string_lossy(), prefix)
}

/// `Fs::temp_file_in(dir, prefix)`: create a unique empty file inside `dir`;
/// `dir` must already exist and be a directory.
pub fn temp_file_in(dir: &str, prefix: Option<&str>) -> Result<String, String> {
    let dir_path = Path::new(dir);
    if !dir_path.is_dir() {
        return Err(format!(
            "cannot create temporary file in '{dir}': not a directory"
        ));
    }
    unique_path(dir_path, prefix.unwrap_or(""), false)
}

/// `Fs::temp_dir_in(dir, prefix)`: create a unique empty directory inside `dir`.
pub fn temp_dir_in(dir: &str, prefix: Option<&str>) -> Result<String, String> {
    let dir_path = Path::new(dir);
    if !dir_path.is_dir() {
        return Err(format!(
            "cannot create temporary directory in '{dir}': not a directory"
        ));
    }
    unique_path(dir_path, prefix.unwrap_or(""), true)
}
