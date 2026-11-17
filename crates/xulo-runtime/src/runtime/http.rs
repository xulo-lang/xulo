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
        xulo_object_set(
            entry,
            c"name".as_ptr(),
            intern_string(h.name.clone()) as i64,
            0,
        );
        xulo_object_set(
            entry,
            c"value".as_ptr(),
            intern_string(h.value.clone()) as i64,
            0,
        );
        xulo_array_push(list, entry as i64, 4);
    }
    xulo_object_set(obj, c"headers".as_ptr(), list as i64, 3);
    xulo_object_set(
        obj,
        c"body".as_ptr(),
        intern_string(res.body.clone()) as i64,
        0,
    );
    fs_ok(obj as i64, 4)
}

/// `List<{name, value}>` 实参：元素是两字段对象，字段都是驻留字符串。
fn header_list(headers: *mut XuloArray) -> Result<Vec<xulo_std::http::Header>, String> {
    let arr = unsafe { &*headers };
    let mut out = Vec::with_capacity(arr.elements.len());
    for (i, &elem) in arr.elements.iter().enumerate() {
        if arr.tags.get(i).copied().map(i64::from).unwrap_or(-1) != 4 {
            return Err("expects `{{ name, value }}` records in the headers list".to_string());
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

// ──────────────────────────────────────────────────────────────────
// 服务器与数据类型（Request/Response/Server）的 AOT 落地
// ──────────────────────────────────────────────────────────────────

fn object_field(obj: *const XuloObject, key: &str) -> Option<(i64, i64)> {
    if obj.is_null() {
        return None;
    }
    unsafe { &*obj }
        .fields
        .iter()
        .enumerate()
        .find(|(_, (k, _))| k == key)
        .map(|(i, (_, v))| {
            let tag = unsafe { &*obj }.field_tags.get(i).copied().unwrap_or(-1);
            (*v, tag)
        })
}

fn object_string(obj: *const XuloObject, key: &str) -> String {
    match object_field(obj, key) {
        Some((0, _)) | None => String::new(),
        Some((ptr, tag)) if tag == 0 || tag == -1 => {
            unsafe { CStr::from_ptr(ptr as *const c_char) }
                .to_str()
                .unwrap_or("")
                .to_string()
        }
        Some((value, 1)) => value.to_string(),
        Some(_) => String::new(),
    }
}

fn header_array(headers: &[xulo_std::http::Header]) -> *mut XuloArray {
    let list = xulo_alloc_array(headers.len());
    for h in headers {
        let entry = xulo_alloc_object(2);
        xulo_object_set(
            entry,
            c"name".as_ptr(),
            intern_string(h.name.clone()) as i64,
            0,
        );
        xulo_object_set(
            entry,
            c"value".as_ptr(),
            intern_string(h.value.clone()) as i64,
            0,
        );
        xulo_array_push(list, entry as i64, 4);
    }
    list
}

fn request_object(req: &xulo_std::http::Request) -> *mut XuloObject {
    let obj = xulo_alloc_object(8);
    let fields = [
        ("method", intern_string(req.method.clone()) as i64, 0),
        ("path", intern_string(req.path.clone()) as i64, 0),
        ("query", header_array(&req.query) as i64, 3),
        ("form", header_array(&req.form) as i64, 3),
        ("headers", header_array(&req.headers) as i64, 3),
        ("body", intern_string(req.body.clone()) as i64, 0),
        ("version", intern_string(req.version.clone()) as i64, 0),
        (
            "remote_addr",
            intern_string(req.remote_addr.clone()) as i64,
            0,
        ),
    ];
    for (name, value, tag) in fields {
        let key = CString::new(name).expect("static field name");
        xulo_object_set(obj, key.as_ptr(), value, tag);
    }
    obj
}

fn response_value_object(res: &xulo_std::http::Response) -> *mut XuloObject {
    let obj = xulo_alloc_object(3);
    xulo_object_set(obj, c"status".as_ptr(), res.status, 1);
    xulo_object_set(
        obj,
        c"headers".as_ptr(),
        header_array(&res.headers) as i64,
        3,
    );
    xulo_object_set(
        obj,
        c"body".as_ptr(),
        intern_string(res.body.clone()) as i64,
        0,
    );
    obj
}

fn request_from_object(obj: *const XuloObject) -> xulo_std::http::Request {
    xulo_std::http::Request {
        method: object_string(obj, "method"),
        path: object_string(obj, "path"),
        query: object_header_list(obj, "query"),
        form: object_header_list(obj, "form"),
        headers: object_header_list(obj, "headers"),
        body: object_string(obj, "body"),
        version: object_string(obj, "version"),
        remote_addr: object_string(obj, "remote_addr"),
    }
}

fn object_header_list(obj: *const XuloObject, key: &str) -> Vec<xulo_std::http::Header> {
    let Some((value, tag)) = object_field(obj, key) else {
        return Vec::new();
    };
    if tag != 3 || value == 0 {
        return Vec::new();
    }
    header_list(value as *mut XuloArray).unwrap_or_default()
}

fn response_from_object(obj: *const XuloObject) -> Result<xulo_std::http::Response, String> {
    let status = object_field(obj, "status").map(|(v, _)| v).unwrap_or(0);
    let headers = object_header_list(obj, "headers");
    let body = object_string(obj, "body");
    Ok(xulo_std::http::Response {
        status,
        headers,
        body,
    })
}

/// Invoke a compiled closure with one pointer argument (`(env, a0..a7) -> i64`).
fn call_closure_one(handler: i64, arg: i64) -> Option<i64> {
    if handler == 0 {
        return None;
    }
    let closure = unsafe { &*(handler as *const XuloClosure) };
    if closure.func.is_null() {
        return None;
    }
    let func: unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64 =
        unsafe { std::mem::transmute(closure.func) };
    Some(unsafe { func(closure.env as i64, arg, 0, 0, 0, 0, 0, 0, 0) })
}

/// `Request(method:, path:, query:, form:, headers:, body:, version:, remote_addr:)`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_http_request_new(
    method: *const c_char,
    path: *const c_char,
    query: *mut XuloArray,
    form: *mut XuloArray,
    headers: *mut XuloArray,
    body: *const c_char,
    version: *const c_char,
    remote_addr: *const c_char,
) -> *mut XuloObject {
    let string = |ptr: *const c_char| -> String {
        if ptr.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(ptr).to_str().unwrap_or("").to_string() }
        }
    };
    let pairs = |arr: *mut XuloArray| -> Vec<xulo_std::http::Header> {
        if arr.is_null() {
            Vec::new()
        } else {
            header_list(arr).unwrap_or_default()
        }
    };
    let req = xulo_std::http::Request {
        method: string(method),
        path: string(path),
        query: pairs(query),
        form: pairs(form),
        headers: pairs(headers),
        body: string(body),
        version: string(version),
        remote_addr: string(remote_addr),
    };
    request_object(&req)
}

/// `Response(status:, headers:, body:)`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_http_response_new(
    status: i64,
    headers: *mut XuloArray,
    body: *const c_char,
) -> *mut XuloObject {
    let headers = if headers.is_null() {
        Vec::new()
    } else {
        header_list(headers).unwrap_or_default()
    };
    let body = if body.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(body).to_str().unwrap_or("").to_string() }
    };
    response_value_object(&xulo_std::http::Response {
        status,
        headers,
        body,
    })
}

/// `Response.text(body[, status])`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_http_response_text(body: *const c_char, status: i64) -> *mut XuloObject {
    let body = if body.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(body).to_str().unwrap_or("").to_string() }
    };
    response_value_object(&xulo_std::http::Response::text(&body, status))
}

/// `Response.json(value)` -> `Result<Response, Error>`
#[unsafe(no_mangle)]
pub extern "C" fn xulo_http_response_json(value: i64, tag: i64) -> *mut XuloEnum {
    match crate::runtime::json::payload_to_json(value, tag, 0)
        .and_then(|json| xulo_std::json::stringify(&json))
    {
        Ok(text) => {
            let res = xulo_std::http::Response::json(&text, 200);
            fs_ok(response_value_object(&res) as i64, 4)
        }
        Err(message) => fs_err(message),
    }
}

/// `std::http::listen(addr)` -> `Result<Server, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_http_listen(addr: *const c_char) -> *mut XuloEnum {
    let addr = if addr.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(addr).to_str().unwrap_or("").to_string() }
    };
    match xulo_std::http::listen(&addr) {
        Ok(server) => {
            let obj = xulo_alloc_object(1);
            let handle = Box::into_raw(Box::new(server));
            xulo_object_set(obj, c"_handle".as_ptr(), handle as i64, 1);
            fs_ok(obj as i64, 4)
        }
        Err(message) => fs_err(message),
    }
}

fn server_handle(server: *const XuloObject) -> Option<&'static xulo_std::http::Server> {
    let (value, _) = object_field(server, "_handle")?;
    if value == 0 {
        return None;
    }
    unsafe { (value as *const xulo_std::http::Server).as_ref() }
}

/// `Server::serve(handler)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
pub extern "C" fn xulo_http_server_serve(server: *mut XuloObject, handler: i64) -> *mut XuloEnum {
    let Some(server) = server_handle(server) else {
        return fs_err("`Server::serve` runs on a Server value".to_string());
    };
    let result = server.serve(|req| {
        let request = request_object(req);
        match call_closure_one(handler, request as i64) {
            Some(answer) => response_from_object(answer as *const XuloObject)
                .unwrap_or_else(|message| xulo_std::http::Response::text(&message, 500)),
            None => xulo_std::http::Response::text("handler is not callable", 500),
        }
    });
    match result {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `Server::addr()` -> `Result<string, Error>`
#[unsafe(no_mangle)]
pub extern "C" fn xulo_http_server_addr(server: *mut XuloObject) -> *mut XuloEnum {
    let Some(server) = server_handle(server) else {
        return fs_err("`Server::addr` runs on a Server value".to_string());
    };
    match server.addr() {
        Ok(addr) => fs_ok(intern_string(addr) as i64, 0),
        Err(message) => fs_err(message),
    }
}

/// `Request.header(name)` / `Request.query_value(name)` / `Request.form_value(name)`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_http_request_lookup(
    request: *mut XuloObject,
    name: *const c_char,
    which: i64,
) -> i64 {
    let name = if name.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(name).to_str().unwrap_or("").to_string() }
    };
    let req = request_from_object(request);
    let found = match which {
        0 => req.header(&name),
        1 => req.query_value(&name),
        _ => req.form_value(&name),
    };
    found.map_or(0, |value| intern_string(value) as i64)
}

/// `Response.header(name)`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_http_response_header(response: *mut XuloObject, name: *const c_char) -> i64 {
    let name = if name.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(name).to_str().unwrap_or("").to_string() }
    };
    match response_from_object(response).map(|res| res.header(&name)) {
        Ok(Some(value)) => intern_string(value) as i64,
        _ => 0,
    }
}
