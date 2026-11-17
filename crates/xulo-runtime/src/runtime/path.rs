use super::*;

/// `std::path::split(path)` -> `{ dir, base, ext }`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_path_split(path: *const c_char) -> *mut XuloObject {
    let path = if path.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() }
    };
    let (dir, base, ext) = xulo_std::path::split(&path);
    let obj = xulo_alloc_object(3);
    xulo_object_set(obj, c"dir".as_ptr(), intern_string(dir) as i64, 0);
    xulo_object_set(obj, c"base".as_ptr(), intern_string(base) as i64, 0);
    xulo_object_set(obj, c"ext".as_ptr(), intern_string(ext) as i64, 0);
    obj
}
