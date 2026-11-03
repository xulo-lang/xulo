use super::*;

// `std::time` 日历里需要运行时堆的 AOT 落地：`parse` 返回内置 Result
// 枚举，`parts` 返回字段对象。`format`/`from_parts`/`weekday` 是标量，
// 在 `xulo-std` 的 ffi 里（域失败与 `Math::to_fixed` 一样终止进程）。

/// `std::time::parse(text)` -> `Result<u64, Error>`（内置 Error：畸形文本
/// 或域外日期都是 Err，绝不 panic）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_time_parse(text: *const c_char) -> *mut XuloEnum {
    let text = if text.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(text).to_str().unwrap_or("").to_string() }
    };
    match xulo_std::time::parse(&text) {
        Ok(ms) => fs_ok(ms as i64, 1),
        Err(message) => fs_err(message),
    }
}

/// `std::time::parts(ms)` -> `{ year, month, day, hour, minute, second,
/// millisecond }`（对每个时间戳都全定义，含域外）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_time_parts(ms: f64) -> *mut XuloObject {
    if !(ms.is_finite() && ms.fract() == 0.0 && ms >= 0.0) {
        eprintln!(
            "`std::time::parts` expects a non-negative integer millisecond count, found {ms}"
        );
        std::process::exit(1);
    }
    let p = xulo_std::time::parts(ms as u64);
    let obj = xulo_alloc_object(7);
    xulo_object_set(obj, c"year".as_ptr(), p.year, 1);
    xulo_object_set(obj, c"month".as_ptr(), p.month, 1);
    xulo_object_set(obj, c"day".as_ptr(), p.day, 1);
    xulo_object_set(obj, c"hour".as_ptr(), p.hour, 1);
    xulo_object_set(obj, c"minute".as_ptr(), p.minute, 1);
    xulo_object_set(obj, c"second".as_ptr(), p.second, 1);
    xulo_object_set(obj, c"millisecond".as_ptr(), p.millisecond, 1);
    obj
}
