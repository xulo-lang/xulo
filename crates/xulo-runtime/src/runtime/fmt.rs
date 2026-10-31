use super::*;

// `std::fmt::format` 的 AOT 落地：模板字符串加 tag 化载荷的参数数组，
// 返回驻留字符串。模板或操作数不合是运行时失败（eprintln + exit(1)，
// 与 `std::time::parts` 的域检查同一形状——参数域按 § 5.3 停下程序）。

/// `std::fmt::format(template, args)` -> 驻留字符串。第二参是调用点的
/// 参数数组指针（缺席由 IR 补 NULL → 空表）。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fmt_format(template: *const c_char, args: *mut XuloArray) -> *const c_char {
    let template = if template.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(template) }
            .to_str()
            .unwrap_or("")
            .to_string()
    };
    let fmt_args: Vec<xulo_std::fmt::FmtArg> = if args.is_null() {
        Vec::new()
    } else {
        let arr = unsafe { &*args };
        arr.elements
            .iter()
            .enumerate()
            .map(|(i, payload)| {
                let tag = arr.tags.get(i).copied().unwrap_or(6);
                payload_to_fmt_arg(*payload, tag)
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

/// (载荷位模式, 标签) → 格式化参数：tag 语义与 `format_value`/print 一致。
/// 整值浮点归一为整数（`2` 与 `2.0` 同值，`%d` 一律可取），其余非标量经
/// `str` 渲染走 `Rendered`。
fn payload_to_fmt_arg(value: i64, tag: i64) -> xulo_std::fmt::FmtArg {
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
