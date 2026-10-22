//! `std::io` 的 AOT 落地：与 `fs` 同构，`Result` 的 Ok/Err 构造共用一套
//! （Err 携带 `Error(message)` 对象，字段 `message` 是字符串）。这里写的是
//! 进程的真实 stdout/stderr、读的是真实 stdin——与解释器的 `print` 输出缓冲
//! 是两回事。

use super::fs::{fs_err, fs_ok};
use super::*;

/// `std::io::print(text)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_io_print(text: *const c_char) -> *mut XuloEnum {
    let text = cstr_arg(text);
    match xulo_std::io::print(&text) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::io::println(text)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_io_println(text: *const c_char) -> *mut XuloEnum {
    let text = cstr_arg(text);
    match xulo_std::io::println(&text) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::io::eprint(text)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_io_eprint(text: *const c_char) -> *mut XuloEnum {
    let text = cstr_arg(text);
    match xulo_std::io::eprint(&text) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::io::eprintln(text)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_io_eprintln(text: *const c_char) -> *mut XuloEnum {
    let text = cstr_arg(text);
    match xulo_std::io::eprintln(&text) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::io::flush()` -> `Result<null, Error>`
#[unsafe(no_mangle)]
pub extern "C" fn xulo_io_flush() -> *mut XuloEnum {
    match xulo_std::io::flush() {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::io::readLine()` -> `Result<string?, Error>`：成功载荷是字符串
/// （tag 0），输入结束是 null（tag 6）——程序据此终止逐行读取。
#[unsafe(no_mangle)]
pub extern "C" fn xulo_io_read_line() -> *mut XuloEnum {
    match xulo_std::io::read_line() {
        Ok(Some(text)) => fs_ok(intern_string(text) as i64, 0),
        Ok(None) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::io::read()` -> `Result<string, Error>`：整个 stdin 读成字符串，
/// 结束时是空串。
#[unsafe(no_mangle)]
pub extern "C" fn xulo_io_read() -> *mut XuloEnum {
    match xulo_std::io::read() {
        Ok(text) => fs_ok(intern_string(text) as i64, 0),
        Err(message) => fs_err(message),
    }
}

/// A C string argument of an `std::io` write: a null pointer reads as the
/// empty string, so a bad caller fails in `xulo_std` rather than here.
fn cstr_arg(text: *const c_char) -> String {
    if text.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(text).to_str().unwrap_or("").to_string() }
    }
}
