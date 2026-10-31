use super::*;

// `std::process` 的 AOT 落地：`Result` 枚举沿用 `fs_ok`/`fs_err`。`exit`
// 不返回（声明 i64 假返回只为让 Cranelift 签名闭合），`args` 产出驻留
// 字符串数组，`run`/`exec` 的可选 input 以 NULL 指针表达（判空 → 不接管
// stdin），可选 timeout 以 f64 传入（缺席时编译器补 -1 → 不限时）。
// `launch`/`Command`/`Child` 没有 AOT 落地：保留的子进程句柄需要对象
// 字段上的原生方法，与 fs 的字节成员一样走解释器。

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

/// `std::process::run(cmd[, input[, timeout]])` ->
/// `Result<{stdout, stderr, code}, Error>`（子进程非零退出是数据，只有
/// spawn 失败/捕获超限/超时才是 Err）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_process_run(
    cmd: *const c_char,
    input: *const c_char,
    timeout: f64,
) -> *mut XuloEnum {
    let cmd = cstr_or_empty(cmd);
    let input = opt_cstr(input);
    match xulo_std::process::run(&cmd, input.as_deref(), opt_timeout(timeout)) {
        Ok(r) => run_output_object(r),
        Err(message) => fs_err(message),
    }
}

/// `std::process::exec(program, args[, input[, timeout]])` -> 同 `run`，但
/// 不经 shell：`args` 的元素就是子进程的 argv（每个元素必须是字符串，
/// 不存在 shell 分词可以藏身）。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_process_exec(
    program: *const c_char,
    args: *mut XuloArray,
    input: *const c_char,
    timeout: f64,
) -> *mut XuloEnum {
    let program = cstr_or_empty(program);
    let argv = match string_list(args) {
        Ok(v) => v,
        Err(message) => return fs_err(message),
    };
    let input = opt_cstr(input);
    match xulo_std::process::exec(&program, &argv, input.as_deref(), opt_timeout(timeout)) {
        Ok(r) => run_output_object(r),
        Err(message) => fs_err(message),
    }
}

/// 捕获记录 → `{stdout, stderr, code}` 载荷对象（`Ok` 分支的 payload）。
fn run_output_object(r: xulo_std::process::RunOutput) -> *mut XuloEnum {
    let obj = xulo_alloc_object(3);
    xulo_object_set(obj, c"stdout".as_ptr(), intern_string(r.stdout) as i64, 0);
    xulo_object_set(obj, c"stderr".as_ptr(), intern_string(r.stderr) as i64, 0);
    xulo_object_set(obj, c"code".as_ptr(), r.code, 1);
    fs_ok(obj as i64, 4)
}

/// NULL 指针 → 空串（`cmd`/`program` 域内最保守的解释）。
fn cstr_or_empty(s: *const c_char) -> String {
    if s.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(s).to_str().unwrap_or("").to_string() }
    }
}

/// NULL 指针 → 不接管 stdin。
fn opt_cstr(s: *const c_char) -> Option<String> {
    if s.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(s).to_str().unwrap_or("").to_string() })
    }
}

/// 可选 timeout：负值/非有限值（编译器为缺席可选参补的 -1）都不限时。
fn opt_timeout(ms: f64) -> Option<u64> {
    if ms.is_finite() && ms >= 0.0 {
        Some(ms as u64)
    } else {
        None
    }
}

/// `List<string>` 实参：元素 tag 0/-1 都是驻留字符串句柄；其他标签是
/// 类型检查漏网的动态值，按参数域错误处理（与解释路径同文案）。
fn string_list(args: *mut XuloArray) -> Result<Vec<String>, String> {
    if args.is_null() {
        return Err("expects a `List<string>` argument, found nothing".to_string());
    }
    let arr = unsafe { &*args };
    let mut strings = Vec::with_capacity(arr.elements.len());
    for (i, &elem) in arr.elements.iter().enumerate() {
        match arr.tags.get(i).copied().unwrap_or(-1) {
            0 | -1 => {
                let s = if elem == 0 {
                    String::new()
                } else {
                    unsafe { CStr::from_ptr(elem as *const c_char) }
                        .to_str()
                        .unwrap_or("")
                        .to_string()
                };
                strings.push(s);
            }
            tag => {
                return Err(format!(
                    "expects string elements in its `List<string>` argument, found tag {tag}"
                ));
            }
        }
    }
    Ok(strings)
}
