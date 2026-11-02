use super::*;

// `std::fmt::format` 的 AOT 落地：模板字符串、tag 化载荷的参数数组，以及
// 每个参数的渲染描述表（Display/Debug 函数地址与用户类型种类）。模板或
// 操作数不合是运行时失败（eprintln + exit(1)，与 `std::time::parts` 的域
// 检查同一形状——参数域按 § 5.3 停下程序）。

/// 一个用户类型的渲染回调地址：`(value, format) -> string`。
type RenderFn = unsafe extern "C" fn(i64, i64) -> i64;

/// `std::fmt::format(template, args, meta)` -> 驻留字符串。
///
/// `args` 是调用点的参数数组（缺席补 NULL → 空表）。`meta` 是每参数四个
/// i64 的描述表：`[kind, display_addr, debug_addr, 0]`，`kind` 为
/// 0=非用户值、1=struct、2=enum；不存在时按纯输出表渲染。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fmt_format(
    template: *const c_char,
    args: *mut XuloArray,
    meta: *mut XuloArray,
) -> *const c_char {
    let template = if template.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(template) }
            .to_str()
            .unwrap_or("")
            .to_string()
    };
    let fmt_args: Vec<xulo_std::fmt::FmtArg<'static>> = if args.is_null() {
        Vec::new()
    } else {
        let arr = unsafe { &*args };
        let meta = if meta.is_null() {
            None
        } else {
            Some(unsafe { &*meta })
        };
        arr.elements
            .iter()
            .enumerate()
            .map(|(i, payload)| {
                let tag = arr.tags.get(i).copied().unwrap_or(6);
                let entry = |slot: usize| -> i64 {
                    meta.as_ref()
                        .and_then(|m| m.elements.get(i * 4 + slot))
                        .copied()
                        .unwrap_or(0)
                };
                let kind = entry(0);
                if kind == 0 {
                    return payload_to_fmt_arg(*payload, tag);
                }
                let display = entry(1);
                let debug = entry(2);
                let value = *payload;
                let kind_name = if kind == 1 { "a struct" } else { "an enum" };
                xulo_std::fmt::FmtArg::User {
                    kind: kind_name,
                    render: Box::new(move |precision, alternate| {
                        let format = make_format(precision, alternate);
                        if display != 0 {
                            return call_render(display, value, format)
                                .map(xulo_std::fmt::FmtText::Consulted);
                        }
                        if debug != 0 {
                            return call_render(debug, value, format)
                                .map(xulo_std::fmt::FmtText::Consulted);
                        }
                        if kind == 1 {
                            return Err("`std::fmt::format` `%v` and `%s` need `Display` or `Debug` for a struct".to_string());
                        }
                        Ok(xulo_std::fmt::FmtText::Table(format_value(value, tag)))
                    }),
                }
            })
            .collect()
    };
    match xulo_std::fmt::format(&template, &fmt_args) {
        Ok(text) => intern_string(text),
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    }
}

/// 构造交给 `Display`/`Debug` 的 `Format`（precision 为 null 或整数，
/// alternate 为 bool）。
fn make_format(precision: Option<usize>, alternate: bool) -> *mut XuloObject {
    let obj = xulo_alloc_object(2);
    xulo_object_set(
        obj,
        c"precision".as_ptr(),
        match precision {
            Some(p) => p as i64,
            None => 0,
        },
        if precision.is_some() { 1 } else { 6 },
    );
    xulo_object_set(
        obj,
        c"alternate".as_ptr(),
        if alternate { 1 } else { 0 },
        5,
    );
    obj
}

/// 调用一个用户渲染函数，读取它返回的 C 字符串。
fn call_render(addr: i64, value: i64, format: *mut XuloObject) -> Result<String, String> {
    if addr == 0 {
        return Err("`std::fmt::format` missing rendering implementation".to_string());
    }
    // # Safety: the address comes from a compiled `Display`/`Debug` function
    // with the `(self, format)` SystemV signature the compiler emits.
    let func: RenderFn = unsafe { std::mem::transmute(addr as usize) };
    let out = unsafe { func(value, format as i64) };
    if out == 0 {
        return Ok(String::new());
    }
    Ok(unsafe { CStr::from_ptr(out as *const c_char) }
        .to_str()
        .unwrap_or("")
        .to_string())
}

/// (载荷位模式, 标签) → 格式化参数：tag 语义与 `format_value`/print 一致。
/// 整值浮点归一为整数，其余非标量经表渲染走 `Rendered`。
fn payload_to_fmt_arg(value: i64, tag: i64) -> xulo_std::fmt::FmtArg<'static> {
    use xulo_std::fmt::FmtArg;
    match tag {
        0 => {
            let ptr = value as *const c_char;
            if ptr.is_null() {
                FmtArg::Null
            } else {
                let text = unsafe { CStr::from_ptr(ptr) }
                    .to_str()
                    .unwrap_or("")
                    .to_string();
                FmtArg::Str(text)
            }
        }
        1 => FmtArg::Int(value),
        2 => {
            let n = f64::from_bits(value as u64);
            // A whole float is its integer, but a negative zero keeps the
            // float slot so `%f` and `%g` can render its sign.
            if n.fract() == 0.0
                && n != 0.0
                && n.is_finite()
                && (i64::MIN as f64..=i64::MAX as f64).contains(&n)
            {
                FmtArg::Int(n as i64)
            } else {
                FmtArg::Float(n)
            }
        }
        5 => FmtArg::Bool(value != 0),
        6 => FmtArg::Null,
        _ => FmtArg::Rendered(format_value(value, tag)),
    }
}
