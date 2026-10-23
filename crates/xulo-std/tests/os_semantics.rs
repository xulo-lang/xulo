//! The `Os` namespace semantics in isolation — no language involved, just the
//! seven primitives and their failure messages. Tests that touch process
//! state (environment, working directory) restore it before finishing, so
//! parallel tests and the host shell stay unaffected.

use xulo_std::os;

#[test]
fn pid_is_the_host_process_id() {
    assert_eq!(os::pid(), i64::from(std::process::id()));
}

#[test]
fn platform_and_arch_are_nonempty_and_total() {
    assert!(!os::platform().is_empty());
    assert!(!os::arch().is_empty());
}

#[test]
fn getenv_roundtrips_through_setenv() {
    let key = format!("XULO_STD_OS_TEST_{}", std::process::id());
    // Start from unset: remove any leftover, then confirm `getenv` sees null.
    // SAFETY: single-threaded test process; the key is a unique plain name.
    unsafe { std::env::remove_var(&key) };
    let before = os::getenv(&key).expect("getenv is infallible for a plain name");
    assert_eq!(before, None, "variable should start unset");

    os::setenv(&key, "hello os").expect("setenv");
    let after = os::getenv(&key).expect("read back");
    assert_eq!(after, Some("hello os".to_string()));

    // SAFETY: cleanup of this test's own variable, same thread.
    unsafe { std::env::remove_var(&key) };
}

#[test]
fn setenv_rejects_a_name_the_os_would_refuse() {
    let err = os::setenv("", "v").expect_err("empty name fails");
    assert!(err.contains("name is empty"), "unexpected: {err}");

    let err = os::setenv("A=B", "v").expect_err("'=' in name fails");
    assert!(err.contains("contains '='"), "unexpected: {err}");

    // A rejected setenv must not have set anything.
    let check = os::getenv("A=B");
    assert!(check.is_ok() && check.unwrap().is_none());
}

#[test]
fn cwd_reports_a_directory_that_exists() {
    let here = os::cwd().expect("cwd");
    assert!(
        std::path::Path::new(&here).is_dir(),
        "not a directory: {here}"
    );
}

#[test]
fn chdir_moves_cwd_and_back() {
    let original = os::cwd().expect("cwd");
    let scratch = std::env::temp_dir();

    os::chdir(&scratch.to_string_lossy()).expect("chdir into temp");
    let moved = os::cwd().expect("cwd after chdir");
    // Compare canonical paths: temp_dir may be a symlink (e.g. /tmp on macOS).
    let moved_canon = std::fs::canonicalize(&moved).unwrap_or_default();
    let scratch_canon = std::fs::canonicalize(&scratch).unwrap_or_default();
    assert_eq!(moved_canon, scratch_canon, "cwd should be the temp dir");

    os::chdir(&original).expect("chdir back");
    assert_eq!(os::cwd().expect("cwd restored"), original);
}

#[test]
fn chdir_to_a_missing_path_fails_with_the_path() {
    let missing = std::env::temp_dir()
        .join(format!("xulo-os-absent-{}", std::process::id()))
        .join("nope");
    let err = os::chdir(&missing.to_string_lossy()).expect_err("missing dir fails");
    assert!(
        err.starts_with("cannot change directory to '") && err.contains("No such file"),
        "unexpected: {err}"
    );
}
