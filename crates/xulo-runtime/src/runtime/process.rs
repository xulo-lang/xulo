use super::*;

// `std::process` 的 AOT 落地：`Result` 枚举沿用 `fs_ok`/`fs_err`。`exit`
// 不返回（声明 i64 假返回只为让 Cranelift 签名闭合），`args` 产出驻留
// 字符串数组，`run` 的可选 input 以 NULL 指针表达（判空 → 不接管 stdin）。

/// `std::process::exit(code)` -> 永不返回（i64 只是占位返回类）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_process_exit(code: f64) -> i64 {
    // `as` 收窄对 NaN/越界饱和成 0——与解释路径的数值收窄一致。
    xulo_std::process::exit(code as i32);
}

/// `std::process::args()` -> `List<string>`（驻留字符串元素）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_process_args() -> *mut XuloArray {
    let arr = xulo_alloc_array(0);
    for arg in xulo_std::process::args() {
        xulo_array_push(arr, intern_string(arg) as i64, 0);
    }
    arr
}

/// `std::process::run(cmd[, input])` -> `Result<{stdout, stderr, code}, Error>`
/// （子进程非零退出是数据，只有 spawn 失败/超限才是 Err）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_process_run(cmd: *const c_char, input: *const c_char) -> *mut XuloEnum {
    let cmd = if cmd.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(cmd).to_str().unwrap_or("").to_string() }
    };
    let input = if input.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(input).to_str().unwrap_or("") }.to_string())
    };
    match xulo_std::process::run(&cmd, input.as_deref()) {
        Ok(r) => {
            let obj = xulo_alloc_object(3);
            xulo_object_set(obj, c"stdout".as_ptr(), intern_string(r.stdout) as i64, 0);
            xulo_object_set(obj, c"stderr".as_ptr(), intern_string(r.stderr) as i64, 0);
            xulo_object_set(obj, c"code".as_ptr(), r.code, 1);
            fs_ok(obj as i64, 4)
        }
        Err(message) => fs_err(message),
    }
}
