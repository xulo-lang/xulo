//! `std::os` 的 AOT 落地：与 `fs`/`io` 同构，`Result` 的 Ok/Err 构造共用一套
//! （Err 携带 `Error(message)` 对象，字段 `message` 是字符串）。`getenv` 的
//! 成功载荷与 `read_line` 一样区分字符串（tag 0）与 null（tag 6，变量未设置）。

use super::fs::{fs_err, fs_ok};
use super::*;

/// `std::os::getenv(name)` -> `Result<string?, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_os_getenv(name: *const c_char) -> *mut XuloEnum {
    let name = cstr_arg(name);
    match xulo_std::os::getenv(&name) {
        Ok(Some(value)) => fs_ok(intern_string(value) as i64, 0),
        Ok(None) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::os::setenv(name, value)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_os_setenv(name: *const c_char, value: *const c_char) -> *mut XuloEnum {
    let name = cstr_arg(name);
    let value = cstr_arg(value);
    match xulo_std::os::setenv(&name, &value) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::os::cwd()` -> `Result<string, Error>`
#[unsafe(no_mangle)]
pub extern "C" fn xulo_os_cwd() -> *mut XuloEnum {
    match xulo_std::os::cwd() {
        Ok(path) => fs_ok(intern_string(path) as i64, 0),
        Err(message) => fs_err(message),
    }
}

/// `std::os::chdir(path)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_os_chdir(path: *const c_char) -> *mut XuloEnum {
    let path = cstr_arg(path);
    match xulo_std::os::chdir(&path) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::os::unsetenv(name)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_os_unsetenv(name: *const c_char) -> *mut XuloEnum {
    let name = cstr_arg(name);
    match xulo_std::os::unsetenv(&name) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::os::tmp_dir()` -> string（总谓词，不失败）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_os_tmp_dir() -> *const c_char {
    intern_string(xulo_std::os::tmp_dir())
}

/// `std::os::home_dir()` / `host_name()` / `user_name()` / `exec_path()` ->
/// `Result<string, Error>`
macro_rules! os_string_result {
    ($name:ident, $call:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn $name() -> *mut XuloEnum {
            match $call() {
                Ok(value) => fs_ok(intern_string(value) as i64, 0),
                Err(message) => fs_err(message),
            }
        }
    };
}

os_string_result!(xulo_os_home_dir, xulo_std::os::home_dir);
os_string_result!(xulo_os_host_name, xulo_std::os::host_name);
os_string_result!(xulo_os_user_name, xulo_std::os::user_name);
os_string_result!(xulo_os_exec_path, xulo_std::os::exec_path);

/// `std::os::cpu_count()` -> number（总谓词，不失败）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_os_cpu_count() -> i64 {
    xulo_std::os::cpu_count()
}

/// `std::os::pid()` -> number（总谓词，不失败）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_os_pid() -> i64 {
    xulo_std::os::pid()
}

/// `std::os::platform()` -> string（总谓词，不失败）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_os_platform() -> *const c_char {
    intern_string(xulo_std::os::platform())
}

/// `std::os::arch()` -> string（总谓词，不失败）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_os_arch() -> *const c_char {
    intern_string(xulo_std::os::arch())
}

/// A C string argument of an `std::os` call: a null pointer reads as the
/// empty string, so a bad caller fails in `xulo_std` rather than here.
fn cstr_arg(text: *const c_char) -> String {
    if text.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(text).to_str().unwrap_or("").to_string() }
    }
}
