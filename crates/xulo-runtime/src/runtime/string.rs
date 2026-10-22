use super::*;

/// 字符串连接
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_concat(a: *const c_char, b: *const c_char) -> *const c_char {
    unsafe {
        let str_a = if a.is_null() {
            ""
        } else {
            CStr::from_ptr(a).to_str().unwrap_or("")
        };

        let str_b = if b.is_null() {
            ""
        } else {
            CStr::from_ptr(b).to_str().unwrap_or("")
        };

        let result = format!("{}{}", str_a, str_b);
        let c_str = CString::new(result).unwrap();
        let ptr = c_str.as_ptr();

        STRINGS.with(|strings| {
            strings.borrow_mut().push(c_str);
        });

        ptr
    }
}

/// 把带类型标签的值格式化成字符串（str() 转换与多参数 print 的参数拼接用）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_stringify(value: i64, tag: i64) -> *const c_char {
    let tag = if tag == -1 {
        xulo_detect_tag(value)
    } else {
        tag
    };
    let s = format_value(value, tag);
    let c_str = CString::new(s).unwrap();
    let ptr = c_str.as_ptr();

    STRINGS.with(|strings| {
        strings.borrow_mut().push(c_str);
    });

    ptr
}

/// `"abc".to_uppercase()`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_to_uppercase(s: *const c_char) -> *const c_char {
    unsafe {
        if s.is_null() {
            return intern_string(String::new());
        }
        let s = CStr::from_ptr(s).to_str().unwrap_or("");
        intern_string(s.to_uppercase())
    }
}
