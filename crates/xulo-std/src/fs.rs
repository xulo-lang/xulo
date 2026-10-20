//! The `Fs` namespace: the five filesystem primitives exposed to Xulo
//! programs. Every fallible operation reports its failure as a message string
//! (`Result<T, Error>` in the language, wrapped by the interpreter with the
//! built-in `Error`); `exists` is a total predicate and never fails.

use std::fs;
use std::path::Path;

/// The failure message of an operation: which operation, on which path, and
/// the underlying OS error (`cannot read 'a.txt': No such file or directory
/// (os error 2)`).
fn fail(op: &str, path: &str, err: std::io::Error) -> String {
    format!("cannot {op} '{path}': {err}")
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

/// `Fs::exists(path)`: whether a file or directory is there. A path that
/// cannot be probed (permission, invalid syntax) counts as absent, so the
/// predicate stays total.
pub fn exists(path: &str) -> bool {
    Path::new(path).exists()
}

/// `Fs::remove(path)`: delete one file or empty directory (non-empty
/// directories are a failure — use `remove` on each entry).
pub fn remove(path: &str) -> Result<(), String> {
    fs::remove_file(path).map_err(|e| fail("remove", path, e))
}

/// `Fs::mkdir(path)`: create the directory, including missing parents
/// (single-level semantics would leave programs to race on `a/b/c`).
pub fn mkdir(path: &str) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| fail("create directory", path, e))
}
