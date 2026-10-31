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

fn cstr_to_string(s: *const c_char) -> String {
    if s.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(s).to_str().unwrap_or("").to_string() }
    }
}

fn cstr_opt(s: *const c_char) -> Option<String> {
    if s.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(s).to_str().unwrap_or("").to_string() })
    }
}

/// `std::fs::append(path, content)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_append(path: *const c_char, content: *const c_char) -> *mut XuloEnum {
    match xulo_std::fs::append(&cstr_to_string(path), &cstr_to_string(content)) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::createNew(path, contents)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_create_new(
    path: *const c_char,
    contents: *const c_char,
) -> *mut XuloEnum {
    match xulo_std::fs::create_new(&cstr_to_string(path), &cstr_to_string(contents)) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::touch(path)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_touch(path: *const c_char) -> *mut XuloEnum {
    match xulo_std::fs::touch(&cstr_to_string(path)) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::setTimes(path, modified)` -> `Result<null, Error>`（毫秒）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_set_times(path: *const c_char, modified: f64) -> *mut XuloEnum {
    let ms = if modified.is_finite() && modified >= 0.0 {
        modified as u64
    } else {
        0
    };
    match xulo_std::fs::set_times(&cstr_to_string(path), ms) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::setMode(path, mode)` -> `Result<null, Error>`（0 …= 4095）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_set_mode(path: *const c_char, mode: f64) -> *mut XuloEnum {
    let mode = if mode.is_finite() && mode >= 0.0 {
        mode as u64
    } else {
        u64::MAX
    };
    match xulo_std::fs::set_mode(&cstr_to_string(path), mode) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::realpath(path)` -> `Result<string, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_realpath(path: *const c_char) -> *mut XuloEnum {
    match xulo_std::fs::realpath(&cstr_to_string(path)) {
        Ok(text) => fs_ok(intern_string(text) as i64, 0),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::stat(path)` -> `Result<{kind, size, modified, mode}, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_stat(path: *const c_char) -> *mut XuloEnum {
    match xulo_std::fs::stat(&cstr_to_string(path)) {
        Ok(st) => {
            let obj = xulo_alloc_object(4);
            xulo_object_set(obj, c"kind".as_ptr(), intern_string(st.kind) as i64, 0);
            xulo_object_set(obj, c"size".as_ptr(), st.size as i64, 1);
            xulo_object_set(obj, c"modified".as_ptr(), st.modified as i64, 1);
            xulo_object_set(obj, c"mode".as_ptr(), st.mode as i64, 1);
            fs_ok(obj as i64, 4)
        }
        Err(message) => fs_err(message),
    }
}

/// `std::fs::copy(from, to)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_copy(from: *const c_char, to: *const c_char) -> *mut XuloEnum {
    match xulo_std::fs::copy(&cstr_to_string(from), &cstr_to_string(to)) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::rename(from, to)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_rename(from: *const c_char, to: *const c_char) -> *mut XuloEnum {
    match xulo_std::fs::rename(&cstr_to_string(from), &cstr_to_string(to)) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::removeDir(path)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_remove_dir(path: *const c_char) -> *mut XuloEnum {
    match xulo_std::fs::remove_dir(&cstr_to_string(path)) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

fn string_list_ok(names: Vec<String>) -> *mut XuloEnum {
    let arr = xulo_alloc_array(names.len());
    for name in names {
        xulo_array_push(arr, intern_string(name) as i64, 0);
    }
    fs_ok(arr as i64, 3)
}

/// `std::fs::readDir(path)` -> `Result<List<string>, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_read_dir(path: *const c_char) -> *mut XuloEnum {
    match xulo_std::fs::read_dir(&cstr_to_string(path)) {
        Ok(names) => string_list_ok(names),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::walk(path)` -> `Result<List<string>, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_walk(path: *const c_char) -> *mut XuloEnum {
    match xulo_std::fs::walk(&cstr_to_string(path)) {
        Ok(paths) => string_list_ok(paths),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::tempFile([prefix])` -> `Result<string, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_temp_file(prefix: *const c_char) -> *mut XuloEnum {
    match xulo_std::fs::temp_file(cstr_opt(prefix).as_deref()) {
        Ok(path) => fs_ok(intern_string(path) as i64, 0),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::tempDir([prefix])` -> `Result<string, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_temp_dir(prefix: *const c_char) -> *mut XuloEnum {
    match xulo_std::fs::temp_dir(cstr_opt(prefix).as_deref()) {
        Ok(path) => fs_ok(intern_string(path) as i64, 0),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::tempFileIn(dir[, prefix])` -> `Result<string, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_temp_file_in(
    dir: *const c_char,
    prefix: *const c_char,
) -> *mut XuloEnum {
    match xulo_std::fs::temp_file_in(&cstr_to_string(dir), cstr_opt(prefix).as_deref()) {
        Ok(path) => fs_ok(intern_string(path) as i64, 0),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::tempDirIn(dir[, prefix])` -> `Result<string, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_temp_dir_in(dir: *const c_char, prefix: *const c_char) -> *mut XuloEnum {
    match xulo_std::fs::temp_dir_in(&cstr_to_string(dir), cstr_opt(prefix).as_deref()) {
        Ok(path) => fs_ok(intern_string(path) as i64, 0),
        Err(message) => fs_err(message),
    }
}
