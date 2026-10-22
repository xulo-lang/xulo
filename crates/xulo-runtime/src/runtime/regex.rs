use super::*;

/// 一次正则匹配结果：普通对象（`value` 字段）+ 旁路登记的捕获组
pub(super) fn regex_match_object(m: &xulo_std::regex::Match) -> *mut XuloObject {
    let value_ptr = intern_string(m.value.clone());
    let obj = xulo_alloc_object(1);
    xulo_object_set(obj, c"value".as_ptr(), value_ptr as i64, 0);
    MATCH_GROUPS.with(|groups| {
        groups.borrow_mut().insert(obj as usize, m.groups.clone());
    });
    obj
}

/// `RegExp::compile(pattern, flags)`：成功返回 `Result::Ok`（payload 为
/// 编译正则指针，tag=9），失败返回 `Result::Err`（payload 为带 `message`
/// 字段的 RegexError 对象）——非法模式与未知标志都是数据
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_regex_compile(
    pattern: *const c_char,
    flags: *const c_char,
) -> *mut XuloEnum {
    unsafe {
        let pattern = if pattern.is_null() {
            String::new()
        } else {
            CStr::from_ptr(pattern).to_str().unwrap_or("").to_string()
        };
        let flags = if flags.is_null() {
            String::new()
        } else {
            CStr::from_ptr(flags).to_str().unwrap_or("").to_string()
        };
        match xulo_std::regex::compile(&pattern, &flags) {
            Ok(re) => {
                let ptr = Box::into_raw(Box::new(re));
                REGEXPS.with(|rs| rs.borrow_mut().push(ptr));
                let e = xulo_alloc_enum(c"Result".as_ptr(), c"Ok".as_ptr(), 0, 1);
                xulo_enum_set(e, 0, ptr as i64, 9);
                e
            }
            Err(message) => {
                let msg_ptr = intern_string(message);
                let err_obj = xulo_alloc_object(1);
                xulo_object_set(err_obj, c"message".as_ptr(), msg_ptr as i64, 0);
                let e = xulo_alloc_enum(c"Result".as_ptr(), c"Err".as_ptr(), 1, 1);
                xulo_enum_set(e, 0, err_obj as i64, 4);
                e
            }
        }
    }
}

/// `re.test(input)` -> bool
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_regex_test(re: i64, input: *const c_char) -> i64 {
    unsafe {
        if re == 0 || input.is_null() {
            return 0;
        }
        let re = &*(re as *const xulo_std::regex::RegExp);
        let input = CStr::from_ptr(input).to_str().unwrap_or("");
        i64::from(re.test(input))
    }
}

/// `re.find(input)` -> 匹配对象或 null
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_regex_find(re: i64, input: *const c_char) -> i64 {
    unsafe {
        if re == 0 || input.is_null() {
            return 0;
        }
        let re = &*(re as *const xulo_std::regex::RegExp);
        let input = CStr::from_ptr(input).to_str().unwrap_or("");
        match re.find(input) {
            Some(m) => regex_match_object(&m) as i64,
            None => 0,
        }
    }
}

/// `re.find_all(input)` -> 匹配对象列表
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_regex_find_all(re: i64, input: *const c_char) -> i64 {
    unsafe {
        if re == 0 || input.is_null() {
            return 0;
        }
        let re = &*(re as *const xulo_std::regex::RegExp);
        let input = CStr::from_ptr(input).to_str().unwrap_or("");
        let ms = re.find_all(input);
        let arr = xulo_alloc_array(ms.len());
        for m in &ms {
            let obj = regex_match_object(m);
            xulo_array_push(arr, obj as i64, 4);
        }
        arr as i64
    }
}

/// `m.group(i)`：i 从 1 起（组 0 不对外，`value` 已携带整个匹配）；
/// 未参与或越界返回 null
#[unsafe(no_mangle)]
pub extern "C" fn xulo_regex_match_group(m: i64, index: i64) -> *const c_char {
    if m == 0 || index < 1 {
        return std::ptr::null();
    }
    let groups = MATCH_GROUPS.with(|g| g.borrow().get(&(m as usize)).cloned());
    let Some(groups) = groups else {
        return std::ptr::null();
    };
    match groups.get((index - 1) as usize).and_then(|g| g.as_ref()) {
        Some(s) => intern_string(s.clone()),
        None => std::ptr::null(),
    }
}

/// `"input".replace(re, "$2.$1")`：模板展开捕获组
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_regex_replace(
    re: i64,
    input: *const c_char,
    template: *const c_char,
) -> *const c_char {
    unsafe {
        if re == 0 || input.is_null() {
            return input;
        }
        let re = &*(re as *const xulo_std::regex::RegExp);
        let input = CStr::from_ptr(input).to_str().unwrap_or("");
        let template = if template.is_null() {
            ""
        } else {
            CStr::from_ptr(template).to_str().unwrap_or("")
        };
        intern_string(re.replace(input, template))
    }
}

/// `"input".replace(re, fn(m) { ... })`：替换函数是 JIT 闭包
/// （入口签名固定 `(env, a0..a7) -> i64`，返回字符串指针）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_regex_replace_with(
    re: i64,
    input: *const c_char,
    closure: i64,
) -> *const c_char {
    type ClosureEntry = extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64;
    unsafe {
        if re == 0 || input.is_null() {
            return input;
        }
        let re = &*(re as *const xulo_std::regex::RegExp);
        let input = CStr::from_ptr(input).to_str().unwrap_or("").to_string();
        let cl = closure as *const XuloClosure;
        if cl.is_null() || (*cl).func.is_null() {
            // 防御：语义层已校验可调用
            return std::ptr::null();
        }
        let func: ClosureEntry = std::mem::transmute((*cl).func);
        let env = (*cl).env as i64;
        let out = re
            .replace_with(&input, |m| {
                let obj = regex_match_object(m);
                let ret = func(env, obj as i64, 0, 0, 0, 0, 0, 0, 0);
                if ret == 0 {
                    Ok::<String, ()>(String::new())
                } else {
                    Ok::<String, ()>(
                        CStr::from_ptr(ret as *const c_char)
                            .to_str()
                            .unwrap_or("")
                            .to_string(),
                    )
                }
            })
            .unwrap_or_default();
        intern_string(out)
    }
}
