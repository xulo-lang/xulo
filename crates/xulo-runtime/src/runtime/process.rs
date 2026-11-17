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
        match arr.tags.get(i).copied().map(i64::from).unwrap_or(-1) {
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

// ──────────────────────────────────────────────────────────────────
// Child 句柄与 Command 规格的 AOT 落地
// ──────────────────────────────────────────────────────────────────

fn process_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(ptr).to_str().unwrap_or("").to_string() }
    }
}

fn process_object_field(obj: *const XuloObject, key: &str) -> Option<(i64, i64)> {
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

fn command_object(spec: xulo_std::process::CommandSpec) -> *mut XuloObject {
    let obj = xulo_alloc_object(1);
    let handle = Box::into_raw(Box::new(spec));
    xulo_object_set(obj, c"_spec".as_ptr(), handle as i64, 1);
    obj
}

fn command_spec(obj: *const XuloObject) -> Option<&'static xulo_std::process::CommandSpec> {
    match process_object_field(obj, "_spec") {
        Some((value, _)) if value != 0 => unsafe {
            (value as *const xulo_std::process::CommandSpec).as_ref()
        },
        _ => None,
    }
}

fn output_enum(result: Result<xulo_std::process::RunOutput, String>) -> *mut XuloEnum {
    match result {
        Ok(r) => run_output_object(r),
        Err(message) => fs_err(message),
    }
}

fn child_object(handle: xulo_std::process::ChildHandle) -> *mut XuloObject {
    let obj = xulo_alloc_object(1);
    let handle = Box::into_raw(Box::new(handle));
    xulo_object_set(obj, c"_child".as_ptr(), handle as i64, 1);
    obj
}

fn child_handle(obj: *mut XuloObject) -> Option<&'static mut xulo_std::process::ChildHandle> {
    match process_object_field(obj, "_child") {
        Some((value, _)) if value != 0 => unsafe {
            (value as *mut xulo_std::process::ChildHandle).as_mut()
        },
        _ => None,
    }
}

/// `std::process::launch(cmd[, input[, timeout]])` -> `Result<Child, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_process_launch(
    cmd: *const c_char,
    input: *const c_char,
    timeout: f64,
) -> *mut XuloEnum {
    let cmd = process_string(cmd);
    let input = (!input.is_null()).then(|| process_string(input));
    match xulo_std::process::launch(&cmd, input.as_deref(), opt_timeout(timeout)) {
        Ok(child) => fs_ok(child_object(child) as i64, 4),
        Err(message) => fs_err(message),
    }
}

/// `std::process::Command::shell(cmd)`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_process_command_shell(cmd: *const c_char) -> *mut XuloObject {
    command_object(xulo_std::process::CommandSpec::shell(&process_string(cmd)))
}

/// `std::process::Command::direct(program, args)`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_process_command_direct(
    program: *const c_char,
    args: *mut XuloArray,
) -> *mut XuloObject {
    let args = if args.is_null() {
        Vec::new()
    } else {
        string_list(args).unwrap_or_default()
    };
    command_object(xulo_std::process::CommandSpec::direct(
        &process_string(program),
        args,
    ))
}

/// `Command::input(text)` / `cwd(path)` / `env(name, value)` / `timeout(ms)`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_process_command_refine(
    spec: *mut XuloObject,
    method: i64,
    a: *const c_char,
    b: *const c_char,
    ms: f64,
) -> *mut XuloObject {
    let Some(spec) = command_spec(spec) else {
        return command_object(xulo_std::process::CommandSpec::shell(""));
    };
    let refined = match method {
        0 => spec.clone().input(&process_string(a)),
        1 => spec.clone().cwd(&process_string(a)),
        2 => spec.clone().env(&process_string(a), &process_string(b)),
        _ => spec.clone().timeout(if ms.is_finite() && ms >= 0.0 {
            ms as u64
        } else {
            0
        }),
    };
    command_object(refined)
}

/// `Command::run()` -> `Result<{stdout, stderr, code}, Error>`
#[unsafe(no_mangle)]
pub extern "C" fn xulo_process_command_run(spec: *mut XuloObject) -> *mut XuloEnum {
    let Some(spec) = command_spec(spec) else {
        return fs_err("`Command::run` runs on a Command value".to_string());
    };
    output_enum(spec.run())
}

/// `Command::launch()` -> `Result<Child, Error>`
#[unsafe(no_mangle)]
pub extern "C" fn xulo_process_command_launch(spec: *mut XuloObject) -> *mut XuloEnum {
    let Some(spec) = command_spec(spec) else {
        return fs_err("`Command::launch` runs on a Command value".to_string());
    };
    match spec.launch() {
        Ok(child) => fs_ok(child_object(child) as i64, 4),
        Err(message) => fs_err(message),
    }
}

/// `Child::pid()` -> number（句柄消耗后仍是事实）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_process_child_pid(child: *mut XuloObject) -> i64 {
    child_handle(child).map(|c| c.pid() as i64).unwrap_or(0)
}

/// `Child::kill()` -> `Result<null, Error>`
#[unsafe(no_mangle)]
pub extern "C" fn xulo_process_child_kill(child: *mut XuloObject) -> *mut XuloEnum {
    match child_handle(child) {
        Some(child) => match child.kill() {
            Ok(()) => fs_ok(0, 6),
            Err(message) => fs_err(message),
        },
        None => fs_err("`Child::kill` runs on a Child value".to_string()),
    }
}

/// `Child::wait()` -> `Result<{stdout, stderr, code}, Error>`
#[unsafe(no_mangle)]
pub extern "C" fn xulo_process_child_wait(child: *mut XuloObject) -> *mut XuloEnum {
    match child_handle(child) {
        Some(child) => output_enum(child.wait()),
        None => fs_err("`Child::wait` runs on a Child value".to_string()),
    }
}
