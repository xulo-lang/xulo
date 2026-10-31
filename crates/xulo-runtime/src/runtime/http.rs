use super::*;

// `std::http` 客户端的 AOT 落地：`Result` 枚举沿用 `fs_ok`/`fs_err`。
// `fetch`/`request` 的可选 headers/body 以 NULL 指针/空指针表达，timeout
// 是 f64 毫秒预算（缺席时编译器补 -1 → 不限时）。响应载荷是
// `{status, headers, body}` 对象（headers 是 `{name, value}` 对象列表）。
// `listen`/`Server`/`Response.text`/构造器没有 AOT 落地：服务器句柄与
// 对象方法只有解释器能挂（与 fs 字节成员、process 的 launch 同列）。

/// `std::http::fetch(url[, timeout])` -> `Result<{status, headers, body}, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_http_fetch(url: *const c_char, timeout: f64) -> *mut XuloEnum {
    let url = if url.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(url).to_str().unwrap_or("").to_string() }
    };
    match xulo_std::http::fetch(&url, opt_timeout(timeout)) {
        Ok(res) => response_object(res),
        Err(message) => fs_err(message),
    }
}

/// `std::http::request(method, url[, headers[, body[, timeout]]])` -> 同
/// `fetch`；headers 是 `{name, value}` 对象列表，body 是文本。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_http_request(
    method: *const c_char,
    url: *const c_char,
    headers: *mut XuloArray,
    body: *const c_char,
    timeout: f64,
) -> *mut XuloEnum {
    let method = if method.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(method).to_str().unwrap_or("").to_string() }
    };
    let url = if url.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(url).to_str().unwrap_or("").to_string() }
    };
    let headers = if headers.is_null() {
        None
    } else {
        match header_list(headers) {
            Ok(h) => Some(h),
            Err(message) => return fs_err(message),
        }
    };
    let body = if body.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(body).to_str().unwrap_or("").to_string() })
    };
    match xulo_std::http::request(
        &method,
        &url,
        headers,
        body.as_deref(),
        opt_timeout(timeout),
    ) {
        Ok(res) => response_object(res),
        Err(message) => fs_err(message),
    }
}

/// 响应记录 → `{status, headers, body}` 载荷对象（`Ok` 的 payload）。
fn response_object(res: xulo_std::http::Response) -> *mut XuloEnum {
    let obj = xulo_alloc_object(3);
    xulo_object_set(obj, c"status".as_ptr(), res.status, 1);
    let list = xulo_alloc_array(res.headers.len());
    for h in &res.headers {
        let entry = xulo_alloc_object(2);
        xulo_object_set(entry, c"name".as_ptr(), intern_string(h.name.clone()) as i64, 0);
        xulo_object_set(entry, c"value".as_ptr(), intern_string(h.value.clone()) as i64, 0);
        xulo_array_push(list, entry as i64, 4);
    }
    xulo_object_set(obj, c"headers".as_ptr(), list as i64, 3);
    xulo_object_set(obj, c"body".as_ptr(), intern_string(res.body.clone()) as i64, 0);
    fs_ok(obj as i64, 4)
}

/// `List<{name, value}>` 实参：元素是两字段对象，字段都是驻留字符串。
fn header_list(headers: *mut XuloArray) -> Result<Vec<xulo_std::http::Header>, String> {
    let arr = unsafe { &*headers };
    let mut out = Vec::with_capacity(arr.elements.len());
    for (i, &elem) in arr.elements.iter().enumerate() {
        if arr.tags.get(i).copied().unwrap_or(-1) != 4 {
            return Err(
                "expects `{{ name, value }}` records in the headers list".to_string()
            );
        }
        let obj = unsafe { &*(elem as *const XuloObject) };
        let field = |key: &str| -> Result<String, String> {
            obj.fields
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| {
                    if *v == 0 {
                        String::new()
                    } else {
                        unsafe { CStr::from_ptr(*v as *const c_char) }
                            .to_str()
                            .unwrap_or("")
                            .to_string()
                    }
                })
                .ok_or_else(|| format!("header record has no `{key}` field"))
        };
        out.push(xulo_std::http::Header {
            name: field("name")?,
            value: field("value")?,
        });
    }
    Ok(out)
}

/// 可选 timeout：负值/非有限值（编译器为缺席可选参补的 -1）都不限时。
fn opt_timeout(ms: f64) -> Option<u64> {
    if ms.is_finite() && ms >= 0.0 {
        Some(ms as u64)
    } else {
        None
    }
}
