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
}

/// 注册字符串指针（用于自动检测，不拥有所有权）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_register_string(ptr: *const c_char) {
    JIT_STRINGS.with(|strings| {
        if !strings.borrow().contains(&ptr) {
            strings.borrow_mut().push(ptr);
        }
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
    let ptr = value as *const c_char;

    // 检查是否是已知字符串（运行时创建）
    let mut found = STRINGS.with(|strings| {
        strings
            .borrow()
            .iter()
            .any(|s| std::ptr::eq(s.as_ptr(), ptr))
    });
    if found {
        return 0;
    }

    // 检查是否是已知字符串（JIT 嵌入）
    found = JIT_STRINGS.with(|strings| strings.borrow().iter().any(|s| std::ptr::eq(*s, ptr)));
    if found {
        return 0;
    }

    // 检查是否是已知数组
    found = ARRAYS.with(|arrays| {
        arrays
            .borrow()
            .iter()
            .any(|a| std::ptr::eq(*a as *const c_char, ptr))
    });
    if found {
        return 3;
    }

    // 检查是否是已知对象
    found = OBJECTS.with(|objects| {
        objects
            .borrow()
            .iter()
            .any(|o| std::ptr::eq(*o as *const c_char, ptr))
    });
    if found {
        return 4;
    }

    // 检查是否是已知枚举值
    found = ENUMS.with(|enums| {
        enums
            .borrow()
            .iter()
            .any(|e| std::ptr::eq(*e as *const c_char, ptr))
    });
    if found {
        return 7;
    }

    // 检查是否是已知闭包值
    found = CLOSURE_PTRS.with(|ps| ps.borrow().contains(&(value as usize)));
    if found {
        return 8;
    }

    // 检查是否是已知编译正则
    found = REGEXPS.with(|rs| {
        rs.borrow()
            .iter()
            .any(|r| std::ptr::eq(*r as *const c_char, ptr))
    });
    if found {
        return 9;
    }

    // 检查是否是已知浮点数
    found = FLOATS.with(|floats| floats.borrow().contains(&value));
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

/// 动态数值转 f64：tag=2 按位模式还原，其余按数值转换。
/// 运行时算术的操作数标签在编译期未知（match 绑定等），统一走这里。
#[unsafe(no_mangle)]
pub extern "C" fn xulo_as_f64(value: i64, tag: i64) -> f64 {
    match tag {
        2 => f64::from_bits(value as u64),
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

/// `<<` / `>>` compiled entry point (spec `expressions/operators.md`): the
/// amount must be in `0 … 63`; an out-of-range amount is a runtime failure,
/// matching the interpreter's `RunError`. `right != 0` selects the
/// sign-filling shift.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_shift(value: i64, amount: i64, right: i64) -> i64 {
    if !(0..64).contains(&amount) {
        eprintln!("shift amount out of range: {amount}");
        std::process::exit(1);
    }
    let amount = amount as u32;
    if right != 0 {
        value >> amount
    } else {
        ((value as u64).wrapping_shl(amount)) as i64
    }
}
