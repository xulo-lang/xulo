use super::*;

/// `std::fs` 的 AOT 落地：`Result` 枚举的 Ok/Err 构造与解释器一致（Err 携带
/// `Error(message)` 对象，字段 `message` 是字符串）。
pub(super) fn fs_ok(value: i64, tag: i64) -> *mut XuloEnum {
    let e = xulo_alloc_enum(c"Result".as_ptr(), c"Ok".as_ptr(), 0, 1);
    xulo_enum_set(e, 0, value, tag);
    e
}

pub(super) fn fs_err(message: String) -> *mut XuloEnum {
    let err_obj = xulo_alloc_object(1);
    xulo_object_set(
        err_obj,
        c"message".as_ptr(),
        intern_string(message) as i64,
        0,
    );
    let e = xulo_alloc_enum(c"Result".as_ptr(), c"Err".as_ptr(), 1, 1);
    xulo_enum_set(e, 0, err_obj as i64, 4);
    e
}

/// `std::fs::read(path)` -> `Result<string, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_read(path: *const c_char) -> *mut XuloEnum {
    let path = if path.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() }
    };
    match xulo_std::fs::read(&path) {
        Ok(text) => fs_ok(intern_string(text) as i64, 0),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::write(path, content)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_write(path: *const c_char, content: *const c_char) -> *mut XuloEnum {
    let path = if path.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() }
    };
    let content = if content.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(content).to_str().unwrap_or("").to_string() }
    };
    match xulo_std::fs::write(&path, &content) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::exists(path)` -> bool（总谓词，不失败）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_exists(path: *const c_char) -> i64 {
    let path = if path.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() }
    };
    i64::from(xulo_std::fs::exists(&path))
}

/// `std::fs::remove(path)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_remove(path: *const c_char) -> *mut XuloEnum {
    let path = if path.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() }
    };
    match xulo_std::fs::remove(&path) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::mkdir(path)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_mkdir(path: *const c_char) -> *mut XuloEnum {
    let path = if path.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() }
    };
    match xulo_std::fs::mkdir(&path) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}
