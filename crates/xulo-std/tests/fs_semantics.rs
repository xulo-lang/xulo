//! The `Fs` namespace semantics in isolation — no language involved, just the
//! five primitives and their failure messages. Fixtures are unique scratch
//! directories under the system temp dir (the crate is zero-dependency, so
//! each test builds and cleans up its own).

use std::fs as std_fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use xulo_std::fs;

/// A fresh scratch directory per call: pid + label + counter keeps parallel
/// tests from colliding.
fn scratch(label: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xulo-std-fs-{}-{}-{}",
        std::process::id(),
        label,
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std_fs::remove_dir_all(&dir);
    std_fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn write_then_read_roundtrips() {
    let dir = scratch("roundtrip");
    let file = dir.join("note.txt");

    fs::write(&path_string(&file), "hello xulo").expect("write");
    let back = fs::read(&path_string(&file)).expect("read");

    assert_eq!(back, "hello xulo");
    let _ = std_fs::remove_dir_all(&dir);
}

#[test]
fn write_truncates_an_existing_file() {
    let dir = scratch("truncate");
    let file = dir.join("log.txt");

    fs::write(&path_string(&file), "old contents that are longer").unwrap();
    fs::write(&path_string(&file), "new").unwrap();
    assert_eq!(fs::read(&path_string(&file)).unwrap(), "new");

    let _ = std_fs::remove_dir_all(&dir);
}

#[test]
fn read_missing_file_reports_path_and_os_error() {
    let dir = scratch("read-missing");
    let file = dir.join("absent.txt");

    let err = fs::read(&path_string(&file)).expect_err("missing file fails");
    assert!(
        err.starts_with("cannot read '") && err.contains("absent.txt"),
        "message should name the operation and path: {err}"
    );
    assert!(
        err.contains("No such file or directory"),
        "message should carry the OS error: {err}"
    );

    let _ = std_fs::remove_dir_all(&dir);
}

#[test]
fn read_of_a_directory_fails() {
    let dir = scratch("read-dir");

    let err = fs::read(&path_string(&dir)).expect_err("a directory is not a string");
    assert!(err.starts_with("cannot read '"), "unexpected: {err}");

    let _ = std_fs::remove_dir_all(&dir);
}

#[test]
fn read_rejects_non_utf8_bytes() {
    let dir = scratch("read-non-utf8");
    let file = dir.join("raw.bin");
    std_fs::write(&file, [0xFF, 0xFE, 0x00, 0x41]).unwrap();

    let err = fs::read(&path_string(&file)).expect_err("invalid UTF-8 fails");
    assert!(err.starts_with("cannot read '"), "unexpected: {err}");

    let _ = std_fs::remove_dir_all(&dir);
}

#[test]
fn write_into_a_missing_parent_fails_but_names_the_path() {
    let dir = scratch("write-missing-parent");
    let file = dir.join("no").join("such").join("dir.txt");

    let err = fs::write(&path_string(&file), "x").expect_err("parent missing");
    assert!(
        err.starts_with("cannot write '") && err.contains("No such file or directory"),
        "unexpected: {err}"
    );

    let _ = std_fs::remove_dir_all(&dir);
}

#[test]
fn exists_reflects_files_directories_and_absence() {
    let dir = scratch("exists");
    let file = dir.join("here.txt");
    std_fs::write(&file, "x").unwrap();

    assert!(fs::exists(&path_string(&file)));
    assert!(fs::exists(&path_string(&dir)));
    assert!(!fs::exists(&path_string(&dir.join("gone.txt"))));

    let _ = std_fs::remove_dir_all(&dir);
}

#[test]
fn remove_deletes_a_file_and_a_missing_path_fails() {
    let dir = scratch("remove");
    let file = dir.join("temp.txt");
    std_fs::write(&file, "x").unwrap();

    fs::remove(&path_string(&file)).expect("remove");
    assert!(!fs::exists(&path_string(&file)));

    let err = fs::remove(&path_string(&file)).expect_err("already gone");
    assert!(
        err.starts_with("cannot remove '") && err.contains("No such file or directory"),
        "unexpected: {err}"
    );

    let _ = std_fs::remove_dir_all(&dir);
}

#[test]
fn remove_refuses_a_non_empty_directory() {
    let dir = scratch("remove-nonempty");
    let sub = dir.join("sub");
    std_fs::create_dir(&sub).unwrap();
    std_fs::write(sub.join("keep.txt"), "x").unwrap();

    let err = fs::remove(&path_string(&sub)).expect_err("not empty");
    assert!(err.starts_with("cannot remove '"), "unexpected: {err}");
    assert!(fs::exists(&path_string(&sub)));

    let _ = std_fs::remove_dir_all(&dir);
}

#[test]
fn mkdir_creates_every_missing_parent() {
    let dir = scratch("mkdir");
    let deep = dir.join("a").join("b").join("c");

    fs::mkdir(&path_string(&deep)).expect("mkdir -p");
    assert!(fs::exists(&path_string(&deep)));

    // Idempotent: a second mkdir on the same path succeeds (create_dir_all).
    fs::mkdir(&path_string(&deep)).expect("mkdir again");

    let _ = std_fs::remove_dir_all(&dir);
}

#[test]
fn mkdir_over_an_existing_file_fails() {
    let dir = scratch("mkdir-over-file");
    let file = dir.join("occupied");
    std_fs::write(&file, "x").unwrap();

    let err = fs::mkdir(&path_string(&file)).expect_err("file in the way");
    assert!(
        err.starts_with("cannot create directory '"),
        "unexpected: {err}"
    );

    let _ = std_fs::remove_dir_all(&dir);
}
