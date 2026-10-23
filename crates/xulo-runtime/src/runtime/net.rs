use super::*;

// `std::net` 的 AOT 落地：与 `fs` 共用 `fs_ok`/`fs_err` 的 `Result` 枚举
// 构造（Err 携带 `Error(message)` 对象）。句柄/长度形参按 f64 接（调用点
// `coerce_to` 对 f64 形参走数值转换，整数字面量与 `Result<number>` 载荷
// 都能正确落地），入口处再收窄成 i64/usize——越界由 `as` 饱和成 0，随后
// 被注册表/长度校验拦成干净的 `Err`。

/// `std::net::tcp_listen(addr)` -> `Result<number, Error>`（监听句柄）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_net_tcp_listen(addr: *const c_char) -> *mut XuloEnum {
    let addr = if addr.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(addr).to_str().unwrap_or("").to_string() }
    };
    match xulo_std::net::tcp_listen(&addr) {
        Ok(handle) => fs_ok(handle, 1),
        Err(message) => fs_err(message),
    }
}

/// `std::net::tcp_accept(listener)` -> `Result<number, Error>`（新流句柄）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_net_tcp_accept(listener: f64) -> *mut XuloEnum {
    match xulo_std::net::tcp_accept(listener as i64) {
        Ok(handle) => fs_ok(handle, 1),
        Err(message) => fs_err(message),
    }
}

/// `std::net::tcp_connect(addr)` -> `Result<number, Error>`（流句柄）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_net_tcp_connect(addr: *const c_char) -> *mut XuloEnum {
    let addr = if addr.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(addr).to_str().unwrap_or("").to_string() }
    };
    match xulo_std::net::tcp_connect(&addr) {
        Ok(handle) => fs_ok(handle, 1),
        Err(message) => fs_err(message),
    }
}

/// `std::net::sock_read(handle, max)` -> `Result<string?, Error>`
/// （`Ok` 载荷 null(6) = EOF，字符串(0) = 数据）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_net_sock_read(handle: f64, max: f64) -> *mut XuloEnum {
    if !(1.0..=16_777_216.0).contains(&max) || !max.is_finite() {
        return fs_err(format!(
            "read size must be between 1 and 16777216, found {max}"
        ));
    }
    match xulo_std::net::sock_read(handle as i64, max as usize) {
        Ok(Some(text)) => fs_ok(intern_string(text) as i64, 0),
        Ok(None) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::net::sock_write(handle, text)` -> `Result<number, Error>`（字节数）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_net_sock_write(handle: f64, text: *const c_char) -> *mut XuloEnum {
    let text = if text.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(text).to_str().unwrap_or("").to_string() }
    };
    match xulo_std::net::sock_write(handle as i64, &text) {
        Ok(count) => fs_ok(count, 1),
        Err(message) => fs_err(message),
    }
}

/// `std::net::sock_shutdown(handle)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
pub extern "C" fn xulo_net_sock_shutdown(handle: f64) -> *mut XuloEnum {
    match xulo_std::net::sock_shutdown(handle as i64) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::net::sock_close(handle)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
pub extern "C" fn xulo_net_sock_close(handle: f64) -> *mut XuloEnum {
    match xulo_std::net::sock_close(handle as i64) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::net::sock_local_addr(handle)` -> `Result<string, Error>`
#[unsafe(no_mangle)]
pub extern "C" fn xulo_net_sock_local_addr(handle: f64) -> *mut XuloEnum {
    match xulo_std::net::sock_local_addr(handle as i64) {
        Ok(addr) => fs_ok(intern_string(addr) as i64, 0),
        Err(message) => fs_err(message),
    }
}

/// `std::net::sock_peer_addr(handle)` -> `Result<string, Error>`
#[unsafe(no_mangle)]
pub extern "C" fn xulo_net_sock_peer_addr(handle: f64) -> *mut XuloEnum {
    match xulo_std::net::sock_peer_addr(handle as i64) {
        Ok(addr) => fs_ok(intern_string(addr) as i64, 0),
        Err(message) => fs_err(message),
    }
}
