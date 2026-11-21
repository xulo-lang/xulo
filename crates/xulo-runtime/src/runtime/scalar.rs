use super::*;

/// 打印整数
#[unsafe(no_mangle)]
pub extern "C" fn xulo_print_int(value: i64) {
    let line = format!("{}", value);
    push_output(line.clone());
    println!("{}", value);
}

/// 打印浮点数
#[unsafe(no_mangle)]
pub extern "C" fn xulo_print_float(value: f64) {
    let line = format!("{}", value);
    push_output(line.clone());
    println!("{}", value);
}

/// 注册浮点数（用于自动检测）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_register_float(bits: i64) {
    FLOATS.with(|floats| {
        if !floats.borrow().contains(&bits) {
            floats.borrow_mut().push(bits);
        }
    });
    FLOAT_VALUES.with(|values| {
        values.borrow_mut().insert(bits);
    });
}

/// 把 `f64` 槽的值按 binary32 舍入并登记，供 `f32` 绑定/算术边界使用。
#[unsafe(no_mangle)]
pub extern "C" fn xulo_round_f32(value: f64) -> f64 {
    let rounded = value as f32 as f64;
    xulo_register_float(rounded.to_bits() as i64);
    rounded
}

/// 注册字符串指针（用于自动检测，不拥有所有权）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_register_string(ptr: *const c_char) {
    JIT_STRINGS.with(|strings| {
        if !strings.borrow().contains(&ptr) {
            strings.borrow_mut().push(ptr);
        }
    });
    JIT_STRING_PTRS.with(|ptrs| {
        ptrs.borrow_mut().insert(ptr as usize);
    });
}

/// 分配一个捕获单元（持有原始位模式）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_alloc_cell(value: i64) -> i64 {
    let cell = Box::into_raw(Box::new(XuloCell { value }));
    CELLS.with(|cells| cells.borrow_mut().push(cell));
    cell as i64
}

/// 读 cell 的原始位模式（空 cell 指针读作 0）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_cell_get(cell: i64) -> i64 {
    if cell == 0 {
        return 0;
    }
    unsafe {
        (cell as *mut XuloCell)
            .as_ref()
            .map(|c| c.value)
            .unwrap_or(0)
    }
}

/// 读 cell 中的 f64 位模式（返回真正的 f64；空 cell 指针读作 0.0）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_cell_get_f64(cell: i64) -> f64 {
    if cell == 0 {
        return 0.0;
    }
    unsafe {
        (cell as *mut XuloCell)
            .as_ref()
            .map(|c| f64::from_bits(c.value as u64))
            .unwrap_or(0.0)
    }
}

/// 写 cell 的原始位模式（空 cell 指针是 no-op）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_cell_set(cell: i64, value: i64) {
    if cell == 0 {
        return;
    }
    unsafe {
        if let Some(c) = (cell as *mut XuloCell).as_mut() {
            c.value = value;
        }
    }
}

/// 分配捕获环境（容纳 len 个 cell 指针）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_alloc_env(len: i64) -> i64 {
    let env = Box::into_raw(Box::new(XuloEnv {
        cells: vec![0; len.max(0) as usize],
    }));
    ENVS.with(|envs| envs.borrow_mut().push(env));
    env as i64
}

/// 向环境槽位写入 cell 指针
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_env_set(env: i64, idx: i64, cell: i64) {
    if env == 0 || idx < 0 {
        return;
    }
    unsafe {
        if let Some(e) = (env as *mut XuloEnv).as_mut()
            && let Some(slot) = e.cells.get_mut(idx as usize)
        {
            *slot = cell;
        }
    }
}

/// 读环境槽位的 cell 指针（空 env / 越界读作 0）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_env_get(env: i64, idx: i64) -> i64 {
    if env == 0 || idx < 0 {
        return 0;
    }
    unsafe {
        (env as *mut XuloEnv)
            .as_ref()
            .and_then(|e| e.cells.get(idx as usize).copied())
            .unwrap_or(0)
    }
}

/// 分配闭包值（函数地址 + 捕获环境）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_alloc_closure(func: i64, env: i64) -> i64 {
    let closure = Box::into_raw(Box::new(XuloClosure {
        func: func as *const std::ffi::c_void,
        env: env as *mut XuloEnv,
    }));
    CLOSURES.with(|cs| cs.borrow_mut().push(closure));
    CLOSURE_PTRS.with(|ps| ps.borrow_mut().insert(closure as usize));
    closure as i64
}

/// 读闭包的函数地址（调用方须先经 xulo_check_callable 校验）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_closure_func(closure: i64) -> i64 {
    unsafe {
        (closure as *mut XuloClosure)
            .as_ref()
            .map(|c| c.func as i64)
            .unwrap_or(0)
    }
}

/// 读闭包的环境指针（调用方须先经 xulo_check_callable 校验）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_closure_env(closure: i64) -> i64 {
    unsafe {
        (closure as *mut XuloClosure)
            .as_ref()
            .map(|c| c.env as i64)
            .unwrap_or(0)
    }
}

/// 校验值是可调用的闭包；否则 panic。返回闭包指针本身。
#[unsafe(no_mangle)]
pub extern "C" fn xulo_check_callable(value: i64) -> i64 {
    let known = CLOSURE_PTRS.with(|ps| ps.borrow().contains(&(value as usize)));
    if value == 0 || !known {
        xulo_panic(value, -1);
        unreachable!();
    }
    value
}

/// 打印函数
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_print(ptr: *const c_char) {
    unsafe {
        if ptr.is_null() {
            push_output(String::new());
            println!();
            return;
        }
        // 尝试作为 C 字符串读取
        let c_str = CStr::from_ptr(ptr);
        if let Ok(s) = c_str.to_str() {
            push_output(s.to_string());
            println!("{}", s);
        } else {
            push_output(String::new());
            println!();
        }
    }
}

/// 带类型标签的打印函数
/// tag: 0=string, 1=int, 2=float, 3=array, 4=object, 5=bool, 6=null, 7=enum, 8=function, -1=自动检测
#[unsafe(no_mangle)]
pub extern "C" fn xulo_print_value(value: i64, tag: i64) {
    let tag = if tag == -1 {
        xulo_detect_tag(value)
    } else {
        tag
    };
    let line = format_value(value, tag);
    push_output(line.clone());
    println!("{}", line);
}

/// 自动检测值的类型标签
/// tag: 0=string, 1=int, 2=float, 3=array, 4=object, 5=bool, 6=null, 7=enum, 8=function
/// 注意：value==0 无法区分 null 和 integer 0，因此不在此处判定 null
#[unsafe(no_mangle)]
pub extern "C" fn xulo_detect_tag(value: i64) -> i64 {
    // 检查是否是已知字符串（运行时创建）
    let mut found = STRING_PTRS.with(|ptrs| ptrs.borrow().contains(&(value as usize)));
    if found {
        return 0;
    }

    // 检查是否是已知字符串（JIT 嵌入）
    found = JIT_STRING_PTRS.with(|ptrs| ptrs.borrow().contains(&(value as usize)));
    if found {
        return 0;
    }

    // 检查是否是已知数组
    found = ARRAY_PTRS.with(|ptrs| ptrs.borrow().contains(&(value as usize)));
    if found {
        return 3;
    }

    // 检查是否是已知对象
    found = OBJECT_PTRS.with(|ptrs| ptrs.borrow().contains(&(value as usize)));
    if found {
        return 4;
    }

    // 检查是否是已知枚举值
    found = ENUM_PTRS.with(|ptrs| ptrs.borrow().contains(&(value as usize)));
    if found {
        return 7;
    }

    // 检查是否是已知闭包值
    found = CLOSURE_PTRS.with(|ps| ps.borrow().contains(&(value as usize)));
    if found {
        return 8;
    }

    // 检查是否是已知编译正则
    found = REGEXP_PTRS.with(|ptrs| ptrs.borrow().contains(&(value as usize)));
    if found {
        return 9;
    }

    // 检查是否是已知浮点数
    found = FLOAT_VALUES.with(|values| values.borrow().contains(&value));
    if found {
        return 2;
    }

    // 默认为整数
    1
}

/// panic 函数
/// tag: 0=string, 1=int, 2=float, 3=array, 4=object, 5=bool, 6=null, -1=自动检测
#[unsafe(no_mangle)]
pub extern "C" fn xulo_panic(value: i64, tag: i64) {
    let tag = if tag == -1 {
        xulo_detect_tag(value)
    } else {
        tag
    };
    eprintln!("panic: {}", format_value(value, tag));
    std::process::exit(1);
}

/// 动态数值转 f64：tag=2 按位模式还原，tag=10 按无符号整数转换，其余按
/// 数值转换。运行时算术的操作数标签在编译期未知（match 绑定等），统一走
/// 这里。
#[unsafe(no_mangle)]
pub extern "C" fn xulo_as_f64(value: i64, tag: i64) -> f64 {
    match tag {
        2 => f64::from_bits(value as u64),
        10 => (value as u64) as f64,
        5 => {
            if value != 0 {
                1.0
            } else {
                0.0
            }
        }
        6 => 0.0,
        _ => value as f64,
    }
}

/// match 字面量 pattern 判定（对齐解释器 `literal_matches`），返回 1/0。
/// scrut_tag/lit_tag: 0=string,1=int,2=float,3=array,4=object,5=bool,6=null,7=enum,-1=运行时检测。
/// value==0 分不清 null 与整数 0：null 字面量按「值为 0 且标签是 null/数字」
/// 判定，与 JIT 既有 `== null` 的判等语义一致。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_literal_match(scrut: i64, scrut_tag: i64, lit: i64, lit_tag: i64) -> i64 {
    let st = if scrut_tag == -1 {
        xulo_detect_tag(scrut)
    } else {
        scrut_tag
    };
    let matched = match lit_tag {
        // 字符串字面量：字符串按内容比对；枚举值按 "Type.Variant" 或变体名
        0 => {
            if scrut == 0 || lit == 0 {
                false
            } else {
                unsafe {
                    let lit_s = CStr::from_ptr(lit as *const c_char);
                    if st == 0 {
                        CStr::from_ptr(scrut as *const c_char) == lit_s
                    } else if st == 7 {
                        let e = scrut as *const XuloEnum;
                        let ty = (*e).type_name;
                        let vn = (*e).variant_name;
                        if ty.is_null() || vn.is_null() {
                            false
                        } else {
                            let ty = CStr::from_ptr(ty);
                            let vn = CStr::from_ptr(vn);
                            let lit_str = lit_s.to_str().unwrap_or("");
                            vn.to_str().unwrap_or("") == lit_str
                                || format!(
                                    "{}.{}",
                                    ty.to_str().unwrap_or(""),
                                    vn.to_str().unwrap_or("")
                                ) == lit_str
                        }
                    } else {
                        false
                    }
                }
            }
        }
        // 数字字面量：scrut 须是数字（int/float 归一成 f64 比较；NaN 自然不等）
        1 | 2 => {
            let lit_f = if lit_tag == 2 {
                f64::from_bits(lit as u64)
            } else {
                lit as f64
            };
            match st {
                1 => (scrut as f64) == lit_f,
                2 => f64::from_bits(scrut as u64) == lit_f,
                _ => false,
            }
        }
        // 布尔字面量：静态标签直接比；动态标签按非零归一（分不清 int 1 与 true）
        5 => (st == 5 || (scrut_tag == -1 && st == 1)) && (scrut != 0) == (lit != 0),
        // null 字面量：值为 0 且确实是 null/数字（0 无法与整数 0 区分）
        6 => scrut == 0 && (st == 1 || st == 6),
        // 列表/对象字面量 pattern 恒不匹配（与解释器一致）
        _ => false,
    };
    i64::from(matched)
}

/// match 区间 pattern 判定（对齐解释器 `pattern_matches` 的 `Range` 分支）：
/// 数值按 `f64` 比较，字符串按文本比较；`start_bits`/`end_bits` 是 f64 位模式。
#[unsafe(no_mangle)]
pub extern "C" fn xulo_range_match(
    value: i64,
    tag: i64,
    start_bits: i64,
    end_bits: i64,
    inclusive: i64,
) -> i64 {
    let st = if tag == -1 {
        xulo_detect_tag(value)
    } else {
        tag
    };
    let start = f64::from_bits(start_bits as u64);
    let end = f64::from_bits(end_bits as u64);
    let inside = if st == 1 || st == 2 || st == 10 {
        let n = match st {
            2 => f64::from_bits(value as u64),
            10 => (value as u64) as f64,
            _ => value as f64,
        };
        if inclusive != 0 {
            n >= start && n <= end
        } else {
            n >= start && n < end
        }
    } else if st == 0 && value != 0 {
        let text = unsafe { CStr::from_ptr(value as *const c_char) };
        let text = text.to_string_lossy();
        let lo = text.as_ref() >= start.to_string().as_str();
        let hi = if inclusive != 0 {
            text.as_ref() <= end.to_string().as_str()
        } else {
            text.as_ref() < end.to_string().as_str()
        };
        lo && hi
    } else {
        false
    };
    i64::from(inside)
}

/// match 类型 pattern 判定（对齐解释器 `value_is_named_type`）。`tag` 是调用点
/// 传来的静态/运行时标签，-1 时现场检测；`optional` 让 `T?` 也接受 null。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_type_match(
    value: i64,
    tag: i64,
    type_name: *const c_char,
    optional: i64,
) -> i64 {
    let st = if tag == -1 {
        xulo_detect_tag(value)
    } else {
        tag
    };
    if optional != 0 && st == 6 {
        return 1;
    }
    if type_name.is_null() {
        return 0;
    }
    let name = unsafe { CStr::from_ptr(type_name) }.to_str().unwrap_or("");
    let matched = match name {
        "number" => matches!(st, 1 | 2 | 10),
        "int" | "i64" | "isize" => st == 1,
        "i8" => st == 1 && i8::try_from(value).is_ok(),
        "i16" => st == 1 && i16::try_from(value).is_ok(),
        "i32" => st == 1 && i32::try_from(value).is_ok(),
        "u64" | "usize" => st == 10 || (st == 1 && value >= 0),
        "u8" => st == 1 && u8::try_from(value).is_ok(),
        "u16" => st == 1 && u16::try_from(value).is_ok(),
        "u32" => st == 1 && u32::try_from(value).is_ok(),
        "float" | "f32" | "f64" => st == 2,
        "string" => st == 0,
        "bool" => st == 5,
        "null" => st == 6,
        "List" => st == 3,
        "Map" => st == 4,
        "unknown" => true,
        _ => {
            if st != 7 || value == 0 {
                false
            } else {
                let e = value as *const XuloEnum;
                unsafe {
                    if (*e).type_name.is_null() {
                        false
                    } else {
                        CStr::from_ptr((*e).type_name).to_str().unwrap_or("") == name
                    }
                }
            }
        }
    };
    i64::from(matched)
}

thread_local! {
    /// 当前直接调用的实参运行时标签。调用点在实参求值后写入，被调函数在
    /// 入口读取 `unknown` 形参的标签；递归调用各自在入口先复制，互不干扰。
    static CALL_TAGS: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
}

/// 写入当前调用帧第 `index` 个实参的运行时标签。
#[unsafe(no_mangle)]
pub extern "C" fn xulo_set_call_tag(index: i64, tag: i64) {
    CALL_TAGS.with(|tags| {
        let mut tags = tags.borrow_mut();
        let index = index.max(0) as usize;
        if tags.len() <= index {
            tags.resize(index + 1, -1);
        }
        tags[index] = tag;
    });
}

/// 读取当前调用帧第 `index` 个实参的标签（缺省 -1，交给运行时检测）。
#[unsafe(no_mangle)]
pub extern "C" fn xulo_call_tag(index: i64) -> i64 {
    CALL_TAGS.with(|tags| {
        tags.borrow()
            .get(index.max(0) as usize)
            .copied()
            .unwrap_or(-1)
    })
}

/// 清空实参标签；间接调用前使用，避免读到上一次调用的残留。
#[unsafe(no_mangle)]
pub extern "C" fn xulo_clear_call_tags() {
    CALL_TAGS.with(|tags| tags.borrow_mut().clear());
}

/// Abort the compiled program with the interpreter's runtime-failure status.
/// A message on stderr plus exit status 1 is the contract Rust callers (and
/// `xupm run`) already use for `xulo_panic`.
fn trap(message: &str) -> ! {
    eprintln!("panic: {message}");
    std::process::exit(1);
}

/// Narrow an exact 128-bit integer result back into a 64-bit slot: the bits
/// are the same for `int` and `u64`; only the print tag differs. A result
/// outside `-2^63 … 2^64-1` is an overflow.
fn narrow_integer(value: i128) -> i64 {
    if i64::try_from(value).is_ok() || u64::try_from(value).is_ok() {
        value as i64
    } else {
        trap("integer overflow")
    }
}

/// Checked integral `+` (spec `types/primitive-types.md`, "Overflow"). The
/// third argument selects the unsigned `u64` domain; the slot stays 64 bits.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_int_add(left: i64, right: i64, unsigned: i64) -> i64 {
    if unsigned != 0 {
        (left as u64)
            .checked_add(right as u64)
            .map(|v| v as i64)
            .unwrap_or_else(|| trap("integer overflow"))
    } else {
        left.checked_add(right)
            .unwrap_or_else(|| trap("integer overflow"))
    }
}

/// Checked integral `-` (a `u64` difference below zero traps, like `int`).
#[unsafe(no_mangle)]
pub extern "C" fn xulo_int_sub(left: i64, right: i64, unsigned: i64) -> i64 {
    if unsigned != 0 {
        (left as u64)
            .checked_sub(right as u64)
            .map(|v| v as i64)
            .unwrap_or_else(|| trap("integer overflow"))
    } else {
        left.checked_sub(right)
            .unwrap_or_else(|| trap("integer overflow"))
    }
}

/// Checked integral `*`.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_int_mul(left: i64, right: i64, unsigned: i64) -> i64 {
    if unsigned != 0 {
        (left as u64)
            .checked_mul(right as u64)
            .map(|v| v as i64)
            .unwrap_or_else(|| trap("integer overflow"))
    } else {
        left.checked_mul(right)
            .unwrap_or_else(|| trap("integer overflow"))
    }
}

/// Checked integral `/`: division truncates toward zero and a zero divisor is
/// a runtime failure (spec `expressions/operators.md`).
#[unsafe(no_mangle)]
pub extern "C" fn xulo_int_div(left: i64, right: i64, unsigned: i64) -> i64 {
    if unsigned != 0 {
        (left as u64)
            .checked_div(right as u64)
            .map(|v| v as i64)
            .unwrap_or_else(|| trap("division by zero"))
    } else {
        left.checked_div(right).unwrap_or_else(|| {
            if right == 0 {
                trap("division by zero")
            } else {
                trap("integer overflow")
            }
        })
    }
}

/// Checked integral `%`: the remainder takes the dividend's sign.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_int_rem(left: i64, right: i64, unsigned: i64) -> i64 {
    if unsigned != 0 {
        (left as u64)
            .checked_rem(right as u64)
            .map(|v| v as i64)
            .unwrap_or_else(|| trap("division by zero"))
    } else {
        left.checked_rem(right).unwrap_or_else(|| {
            if right == 0 {
                trap("division by zero")
            } else {
                trap("integer overflow")
            }
        })
    }
}

/// Checked integral `**`; a negative exponent is a runtime failure.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_int_pow(left: i64, right: i64, unsigned: i64) -> i64 {
    let Ok(exponent) = u32::try_from(right) else {
        trap("negative exponent");
    };
    if unsigned != 0 {
        (left as u64)
            .checked_pow(exponent)
            .map(|v| v as i64)
            .unwrap_or_else(|| trap("integer overflow"))
    } else {
        left.checked_pow(exponent)
            .unwrap_or_else(|| trap("integer overflow"))
    }
}

/// Checked unary `-`. `-u64` narrows to `int`, like the interpreter.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_int_neg(value: i64, unsigned: i64) -> i64 {
    if unsigned != 0 {
        narrow_integer(-((value as u64) as i128))
    } else {
        value
            .checked_neg()
            .unwrap_or_else(|| trap("integer overflow"))
    }
}

/// Exact integral comparison across signedness (spec
/// `expressions/operators.md`, "Comparison operators"): `int` and `u64`
/// compare by mathematical value, so `0 - 1 == 18446744073709551615` is
/// false. Returns `-1`, `0`, or `1`.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_num_compare(
    left: i64,
    right: i64,
    left_unsigned: i64,
    right_unsigned: i64,
) -> i64 {
    let l = if left_unsigned != 0 {
        (left as u64) as i128
    } else {
        left as i128
    };
    let r = if right_unsigned != 0 {
        (right as u64) as i128
    } else {
        right as i128
    };
    match l.cmp(&r) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// `<<` / `>>` compiled entry point (spec `expressions/operators.md`): the
/// amount must be in `0 … 63`; an out-of-range amount is a runtime failure,
/// matching the interpreter's `RunError`. `mode` is 0 for `<<`, 1 for a
/// sign-filling `>>` (`int`), and 2 for a zero-filling `>>` (`u64`).
#[unsafe(no_mangle)]
pub extern "C" fn xulo_shift(value: i64, amount: i64, mode: i64) -> i64 {
    if !(0..64).contains(&amount) {
        eprintln!("shift amount out of range: {amount}");
        std::process::exit(1);
    }
    let amount = amount as u32;
    match mode {
        0 => ((value as u64).wrapping_shl(amount)) as i64,
        2 => ((value as u64) >> amount) as i64,
        _ => value >> amount,
    }
}
