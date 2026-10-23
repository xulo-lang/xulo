//! The `Os` namespace: process and host identity, the working directory, and
//! the environment — the small surface a program needs to talk to the OS it
//! runs on. Every fallible operation reports its failure as a message string
//! (`Result<T, Error>` in the language); `pid`, `platform`, and `arch` are
//! total and never fail. Like `Fs` and `Io`, this namespace is
//! implementation-first (the language itself has no OS access — the host
//! runtime supplies it).

use std::env;

/// `Os::getenv(name)`: the value of an environment variable, or
/// `Result::Ok(null)` when the variable is unset — the same optional-string
/// shape `std::io::readLine` uses for end-of-stream. A variable whose bytes
/// are not valid UTF-8 is a failure, not a lie of `null`.
pub fn getenv(name: &str) -> Result<Option<String>, String> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(format!(
            "cannot read environment variable '{name}': value is not valid UTF-8"
        )),
    }
}

/// `Os::setenv(name, value)`: set an environment variable for this process.
/// A name that is empty or contains `=` is a failure — the OS would reject
/// it, and the host must not panic on a program's mistake.
pub fn setenv(name: &str, value: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("cannot set environment variable: name is empty".to_string());
    }
    if name.contains('=') {
        return Err(format!(
            "cannot set environment variable '{name}': name contains '='"
        ));
    }
    if name.contains('\0') || value.contains('\0') {
        return Err(format!(
            "cannot set environment variable '{name}': name or value contains a NUL byte"
        ));
    }
    // SAFETY: the name and value are validated above (no empty name, no `=`,
    // no NUL). `set_var` is `unsafe` in Rust 2024 only because another OS
    // thread could observe the environment mid-update; Xulo tasks are
    // cooperative green threads on one OS thread, so no concurrent reader
    // can see a torn environment.
    unsafe { env::set_var(name, value) };
    Ok(())
}

/// `Os::cwd()`: the process's current working directory as a string.
pub fn cwd() -> Result<String, String> {
    env::current_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|e| format!("cannot get current directory: {e}"))
}

/// `Os::chdir(path)`: change the process's current working directory.
pub fn chdir(path: &str) -> Result<(), String> {
    env::set_current_dir(path).map_err(|e| format!("cannot change directory to '{path}': {e}"))
}

/// `Os::pid()`: the process id. Total — every process has one.
pub fn pid() -> i64 {
    i64::from(std::process::id())
}

/// `Os::platform()`: the operating-system family — `"linux"`, `"macos"`,
/// `"windows"`, … Total, because the program is always running on one.
pub fn platform() -> &'static str {
    env::consts::OS
}

/// `Os::arch()`: the CPU architecture — `"x86_64"`, `"aarch64"`, … Total,
/// for the same reason as [`platform`].
pub fn arch() -> &'static str {
    env::consts::ARCH
}
