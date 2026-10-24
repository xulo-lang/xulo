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

/// Read a string argument off the C boundary: null and invalid UTF-8 both
/// read as `""` (the same lenience as `xulo_string_to_uppercase`).
unsafe fn str_arg(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p).to_str().unwrap_or("").to_string() }
}

/// `trim()`: the receiver with leading and trailing whitespace removed
/// (Unicode whitespace, both ends — Rust's `str::trim`).
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_trim(s: *const c_char) -> *const c_char {
    let s = unsafe { str_arg(s) };
    intern_string(s.trim())
}

/// `to_lowercase()`: Unicode-lowercased copy of the receiver.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_to_lowercase(s: *const c_char) -> *const c_char {
    let s = unsafe { str_arg(s) };
    intern_string(s.to_lowercase())
}

/// `length()`: the number of Unicode scalar values (characters, not bytes).
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_length(s: *const c_char) -> i64 {
    let s = unsafe { str_arg(s) };
    s.chars().count() as i64
}

/// `contains(needle)`: whether the receiver contains the substring (an empty
/// needle is contained in every string).
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_contains(s: *const c_char, needle: *const c_char) -> i64 {
    let s = unsafe { str_arg(s) };
    let needle = unsafe { str_arg(needle) };
    s.contains(&needle) as i64
}

/// `starts_with(prefix)`: whether the receiver begins with the prefix.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_starts_with(s: *const c_char, prefix: *const c_char) -> i64 {
    let s = unsafe { str_arg(s) };
    let prefix = unsafe { str_arg(prefix) };
    s.starts_with(&prefix) as i64
}

/// `ends_with(suffix)`: whether the receiver ends with the suffix.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_ends_with(s: *const c_char, suffix: *const c_char) -> i64 {
    let s = unsafe { str_arg(s) };
    let suffix = unsafe { str_arg(suffix) };
    s.ends_with(&suffix) as i64
}

/// `split(sep)`: the pieces between occurrences of `sep` (a piece may be
/// empty; a `sep` that never occurs yields one piece — the whole string). An
/// empty `sep` splits into single characters.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_split(s: *const c_char, sep: *const c_char) -> *mut XuloArray {
    let s = unsafe { str_arg(s) };
    let sep = unsafe { str_arg(sep) };
    let arr = xulo_alloc_array(0);
    if sep.is_empty() {
        for ch in s.chars() {
            let piece = intern_string(ch.to_string());
            xulo_array_push(arr, piece as i64, 0);
        }
    } else {
        for part in s.split(&sep) {
            let piece = intern_string(part);
            xulo_array_push(arr, piece as i64, 0);
        }
    }
    arr
}

/// Shared body of `pad_start` / `pad_end`: space-fill the receiver out to
/// `width` characters (width counts Unicode scalar values); a receiver that
/// is already at least `width` long is returned unchanged.
fn pad_impl(s: &str, width: i64, method: &str, pad_left: bool) -> *const c_char {
    let max = super::MAX_STRING_GROWTH;
    if width > max {
        eprintln!("`{method}` width must be at most {max}, found {width}");
        std::process::exit(1);
    }
    let width = width.max(0) as usize;
    let len = s.chars().count();
    if len >= width {
        return intern_string(s.to_string());
    }
    let fill = " ".repeat(width - len);
    intern_string(if pad_left {
        format!("{fill}{s}")
    } else {
        format!("{s}{fill}")
    })
}

/// `pad_start(width)`: left-pad with spaces to `width` characters.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_pad_start(s: *const c_char, width: i64) -> *const c_char {
    let s = unsafe { str_arg(s) };
    pad_impl(&s, width, "pad_start", true)
}

/// `pad_end(width)`: right-pad with spaces to `width` characters.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_pad_end(s: *const c_char, width: i64) -> *const c_char {
    let s = unsafe { str_arg(s) };
    pad_impl(&s, width, "pad_end", false)
}

/// `repeat(n)`: the receiver concatenated with itself `n` times; `n <= 0`
/// yields `""` (fractional counts truncate toward zero at the call site's
/// integer coercion).
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_repeat(s: *const c_char, n: i64) -> *const c_char {
    let s = unsafe { str_arg(s) };
    let max = super::MAX_STRING_GROWTH;
    if n > max {
        eprintln!("`repeat` count must be at most {max}, found {n}");
        std::process::exit(1);
    }
    if n <= 0 {
        return intern_string(String::new());
    }
    intern_string(s.repeat(n as usize))
}

/// `stringSize(s)`: the number of Unicode scalar values in `s`.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_size(s: *const c_char) -> i64 {
    let s = unsafe { str_arg(s) };
    s.chars().count() as i64
}

/// `stringCodeAt(s, i)`: the Unicode scalar value at index `i`; an index
/// outside `0 … stringSize(s) − 1` is a runtime failure (spec
/// `memory-and-runtime.md`, *String index out of range*).
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_code_at(s: *const c_char, i: i64) -> i64 {
    let s = unsafe { str_arg(s) };
    if i < 0 {
        eprintln!("string index out of range: {i}");
        std::process::exit(1);
    }
    match s.chars().nth(i as usize) {
        Some(ch) => ch as u32 as i64,
        None => {
            eprintln!("string index out of range: {i}");
            std::process::exit(1);
        }
    }
}

/// `stringSlice(s, start, end)`: the scalars at indices `start … end − 1`.
/// Bounds outside `0 … stringSize(s)` or `start > end` is a runtime failure.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_slice(s: *const c_char, start: i64, end: i64) -> *const c_char {
    let s = unsafe { str_arg(s) };
    let len = s.chars().count() as i64;
    if start < 0 || end < start || end > len {
        eprintln!("string slice out of range: {start}..{end}");
        std::process::exit(1);
    }
    let out: String = s
        .chars()
        .skip(start as usize)
        .take((end - start) as usize)
        .collect();
    intern_string(out)
}

/// `stringBytes(s)`: the UTF-8 bytes of `s` as a `List<u8>` (each byte is an
/// `int`-tagged element).
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_bytes(s: *const c_char) -> *mut XuloArray {
    let s = unsafe { str_arg(s) };
    let arr = xulo_alloc_array(s.len());
    for byte in s.as_bytes() {
        xulo_array_push(arr, *byte as i64, 1);
    }
    arr
}
