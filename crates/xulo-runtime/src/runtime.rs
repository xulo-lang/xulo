use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use crate::value::format_number;

/// xulo 对象
#[repr(C)]
pub struct XuloObject {
    pub fields: Vec<(String, i64)>,
    pub field_tags: Vec<i64>,
}

/// xulo 数组
#[repr(C)]
pub struct XuloArray {
    pub elements: Vec<i64>,
    pub tags: Vec<i64>,
}

/// xulo 枚举值：变体序号 + 供打印的名字 + payload 槽（值与类型标签）。
/// 名字指针指向编译产物里的静态字符串，生命周期与进程相同。
#[repr(C)]
pub struct XuloEnum {
    pub variant: i64,
    pub variant_name: *const c_char,
    pub type_name: *const c_char,
    pub elements: Vec<i64>,
    pub tags: Vec<i64>,
}

/// 闭包捕获单元：持有被装箱变量的原始位模式。
/// 标签不入 cell——读出时与未装箱路径一样按槽位/伴随标签推断。
#[repr(C)]
pub struct XuloCell {
    pub value: i64,
}

/// 闭包捕获环境：按自由变量的确定顺序容纳 cell 指针。
#[repr(C)]
pub struct XuloEnv {
    pub cells: Vec<i64>,
}

/// 闭包值：目标函数地址 + 捕获环境。函数地址是编译产物里闭包入口的
/// 实地址（JIT 下是 finalize 后的机器码指针，AOT 下经链接器解析）。
#[repr(C)]
pub struct XuloClosure {
    pub func: *const std::ffi::c_void,
    pub env: *mut XuloEnv,
}

thread_local! {
    static OBJECTS: RefCell<Vec<*mut XuloObject>> = const { RefCell::new(Vec::new()) };
    static ENUMS: RefCell<Vec<*mut XuloEnum>> = const { RefCell::new(Vec::new()) };
    static ARRAYS: RefCell<Vec<*mut XuloArray>> = const { RefCell::new(Vec::new()) };
    static STRINGS: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };
    static FLOATS: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
    /// JIT 嵌入的字符串指针（仅记录地址，不拥有所有权）
    static JIT_STRINGS: RefCell<Vec<*const c_char>> = const { RefCell::new(Vec::new()) };
    /// 输出缓冲区（用于测试捕获 print 输出）
    static OUTPUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// 活跃的闭包捕获单元
    static CELLS: RefCell<Vec<*mut XuloCell>> = const { RefCell::new(Vec::new()) };
    /// 活跃的捕获环境
    static ENVS: RefCell<Vec<*mut XuloEnv>> = const { RefCell::new(Vec::new()) };
    /// 活跃的闭包值（清理所有者）
    static CLOSURES: RefCell<Vec<*mut XuloClosure>> = const { RefCell::new(Vec::new()) };
    /// 闭包指针集合：O(1) 判定一个 i64 是否是可调用闭包（detect/check）
    static CLOSURE_PTRS: RefCell<std::collections::HashSet<usize>> = RefCell::new(std::collections::HashSet::new());
    /// 活跃的编译正则（JIT/AOT 侧 RegExp 值的所有者）
    static REGEXPS: RefCell<Vec<*mut xulo_std::regex::RegExp>> = const { RefCell::new(Vec::new()) };
    /// 正则匹配对象的捕获组（match 对象本身是普通 XuloObject，
    /// `group(i)` 经此表取回组内容）
    static MATCH_GROUPS: RefCell<std::collections::HashMap<usize, Vec<Option<String>>>> =
        RefCell::new(std::collections::HashMap::new());
}

/// 取走输出缓冲区（测试用）
pub fn xulo_take_output() -> Vec<String> {
    OUTPUT.with(|out| out.borrow_mut().drain(..).collect())
}

/// 将一行文本推入输出缓冲区
fn push_output(line: String) {
    OUTPUT.with(|out| out.borrow_mut().push(line));
}

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

fn format_value(value: i64, tag: i64) -> String {
    match tag {
        0 => {
            let ptr = value as *const c_char;
            unsafe {
                if ptr.is_null() {
                    "null".to_string()
                } else {
                    match CStr::from_ptr(ptr).to_str() {
                        Ok(s) => s.to_string(),
                        Err(_) => "null".to_string(),
                    }
                }
            }
        }
        1 => format!("{}", value),
        2 => format_number(f64::from_bits(value as u64)),
        3 => {
            let arr = value as *const XuloArray;
            unsafe {
                if arr.is_null() {
                    "[]".to_string()
                } else {
                    let elements = &(*arr).elements;
                    let tags = &(*arr).tags;
                    let strs: Vec<String> = elements
                        .iter()
                        .enumerate()
                        .map(|(i, v)| {
                            let t = if i < tags.len() { tags[i] } else { 1 };
                            format_value(*v, t)
                        })
                        .collect();
                    format!("[{}]", strs.join(", "))
                }
            }
        }
        4 => {
            let obj = value as *const XuloObject;
            unsafe {
                if obj.is_null() {
                    "{}".to_string()
                } else {
                    let fields = &(*obj).fields;
                    let field_tags = &(*obj).field_tags;
                    let strs: Vec<String> = fields
                        .iter()
                        .enumerate()
                        .map(|(i, (k, v))| {
                            let t = if i < field_tags.len() {
                                field_tags[i]
                            } else {
                                0
                            };
                            format!("{}: {}", k, format_value(*v, t))
                        })
                        .collect();
                    format!("{{ {} }}", strs.join(", "))
                }
            }
        }
        5 => format!("{}", value != 0),
        6 => "null".to_string(),
        8 => "<function>".to_string(),
        9 => "<regex>".to_string(),
        7 => {
            let e = value as *const XuloEnum;
            unsafe {
                if e.is_null() {
                    "null".to_string()
                } else {
                    let cstr = |p: *const c_char| {
                        if p.is_null() {
                            String::new()
                        } else {
                            CStr::from_ptr(p).to_str().unwrap_or("").to_string()
                        }
                    };
                    let ty = cstr((*e).type_name);
                    let vn = cstr((*e).variant_name);
                    let elements = &(*e).elements;
                    let tags = &(*e).tags;
                    if elements.is_empty() {
                        format!("{}.{}", ty, vn)
                    } else if elements.len() == 1 {
                        let t = if tags.is_empty() { 1 } else { tags[0] };
                        format!("{}.{}({})", ty, vn, format_value(elements[0], t))
                    } else {
                        let strs: Vec<String> = elements
                            .iter()
                            .enumerate()
                            .map(|(i, v)| {
                                let t = if i < tags.len() { tags[i] } else { 1 };
                                format_value(*v, t)
                            })
                            .collect();
                        format!("{}.{}([{}])", ty, vn, strs.join(", "))
                    }
                }
            }
        }
        _ => format!("{}", value),
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

/// 分配对象
#[unsafe(no_mangle)]
pub extern "C" fn xulo_alloc_object(size: usize) -> *mut XuloObject {
    let obj = Box::new(XuloObject {
        fields: Vec::with_capacity(size),
        field_tags: Vec::with_capacity(size),
    });
    let ptr = Box::into_raw(obj);

    OBJECTS.with(|objects| {
        objects.borrow_mut().push(ptr);
    });

    ptr
}

/// 分配数组
#[unsafe(no_mangle)]
pub extern "C" fn xulo_alloc_array(size: usize) -> *mut XuloArray {
    let arr = Box::new(XuloArray {
        elements: Vec::with_capacity(size),
        tags: Vec::with_capacity(size),
    });
    let ptr = Box::into_raw(arr);

    ARRAYS.with(|arrays| {
        arrays.borrow_mut().push(ptr);
    });

    ptr
}

/// 分配枚举值（名字指针借用编译产物的静态字符串，不复制）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_alloc_enum(
    type_name: *const c_char,
    variant_name: *const c_char,
    variant: i64,
    len: usize,
) -> *mut XuloEnum {
    let e = Box::new(XuloEnum {
        variant,
        variant_name,
        type_name,
        elements: Vec::with_capacity(len),
        tags: Vec::with_capacity(len),
    });
    let ptr = Box::into_raw(e);
    ENUMS.with(|enums| {
        enums.borrow_mut().push(ptr);
    });
    ptr
}

/// 写入枚举 payload 槽（值 + 类型标签）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_enum_set(e: *mut XuloEnum, index: usize, value: i64, tag: i64) {
    unsafe {
        if !e.is_null() {
            (*e).elements.push(value);
            (*e).tags.push(tag);
            let _ = index; // 槽位按写入顺序即索引，保留参数对齐调用约定
        }
    }
}

/// 枚举值的变体序号（match 的 tag 测试）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_enum_variant(e: *const XuloEnum) -> i64 {
    unsafe { if e.is_null() { -1 } else { (*e).variant } }
}

/// 读取枚举 payload 值
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_enum_get(e: *const XuloEnum, index: usize) -> i64 {
    unsafe {
        if e.is_null() || index >= (*e).elements.len() {
            0
        } else {
            (&(*e).elements)[index]
        }
    }
}

/// 读取枚举 payload 的类型标签
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_enum_get_tag(e: *const XuloEnum, index: usize) -> i64 {
    unsafe {
        if e.is_null() || index >= (*e).tags.len() {
            1
        } else {
            (&(*e).tags)[index]
        }
    }
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

/// 区间表达式求值为列表：从 start 起步长 1，inclusive 决定是否含上界
/// （对齐解释器 `Expression::Range`）。整数值入 int 槽；小数/超大值按 f64
/// 位模式入 float 槽并登记，保证打印与自动检测正确。
#[unsafe(no_mangle)]
pub extern "C" fn xulo_range_list(start: f64, end: f64, inclusive: i64) -> *mut XuloArray {
    let arr = xulo_alloc_array(0);
    let mut i = start;
    loop {
        let has_next = if inclusive != 0 { i <= end } else { i < end };
        if !has_next {
            break;
        }
        if i.is_finite() && i.fract() == 0.0 && i.abs() < 9.007_199_254_740_992e15 {
            xulo_array_push(arr, i as i64, 1);
        } else {
            let bits = i.to_bits() as i64;
            xulo_register_float(bits);
            xulo_array_push(arr, bits, 2);
        }
        let next = i + 1.0;
        if next == i {
            // ±inf 或超出 f64 步进精度：终止死循环（解释器在此会无限循环）
            break;
        }
        i = next;
    }
    arr
}

/// 字符串连接
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_concat(a: *const c_char, b: *const c_char) -> *const c_char {
    unsafe {
        let str_a = if a.is_null() {
            ""
        } else {
            CStr::from_ptr(a).to_str().unwrap_or("")
        };

        let str_b = if b.is_null() {
            ""
        } else {
            CStr::from_ptr(b).to_str().unwrap_or("")
        };

        let result = format!("{}{}", str_a, str_b);
        let c_str = CString::new(result).unwrap();
        let ptr = c_str.as_ptr();

        STRINGS.with(|strings| {
            strings.borrow_mut().push(c_str);
        });

        ptr
    }
}

/// 把带类型标签的值格式化成字符串（str() 转换与多参数 print 的参数拼接用）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_stringify(value: i64, tag: i64) -> *const c_char {
    let tag = if tag == -1 {
        xulo_detect_tag(value)
    } else {
        tag
    };
    let s = format_value(value, tag);
    let c_str = CString::new(s).unwrap();
    let ptr = c_str.as_ptr();

    STRINGS.with(|strings| {
        strings.borrow_mut().push(c_str);
    });

    ptr
}

/// 数组 push（带类型标签）
/// tag: 0=string, 1=int, 2=float, 3=array, 4=object, 5=bool, 6=null, -1=自动检测
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_array_push(arr: *mut XuloArray, value: i64, tag: i64) {
    unsafe {
        if !arr.is_null() {
            let tag = if tag == -1 {
                xulo_detect_tag(value)
            } else {
                tag
            };
            (*arr).elements.push(value);
            (*arr).tags.push(tag);
        }
    }
}

/// 数组获取标签
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_array_get_tag(arr: *const XuloArray, index: usize) -> i64 {
    unsafe {
        if arr.is_null() || index >= (*arr).tags.len() {
            1
        } else {
            (&(*arr).tags)[index]
        }
    }
}

/// 数组连接：返回一个新数组，包含 a 的所有元素后跟 b 的所有元素
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_array_concat(a: *const XuloArray, b: *const XuloArray) -> *mut XuloArray {
    unsafe {
        let len_a = if a.is_null() { 0 } else { (*a).elements.len() };
        let len_b = if b.is_null() { 0 } else { (*b).elements.len() };
        let result = xulo_alloc_array(len_a + len_b);
        if !result.is_null() {
            if !a.is_null() {
                for i in 0..len_a {
                    let val = (&(*a).elements)[i];
                    let tag = if i < (*a).tags.len() {
                        (&(*a).tags)[i]
                    } else {
                        1
                    };
                    xulo_array_push(result, val, tag);
                }
            }
            if !b.is_null() {
                for i in 0..len_b {
                    let val = (&(*b).elements)[i];
                    let tag = if i < (*b).tags.len() {
                        (&(*b).tags)[i]
                    } else {
                        1
                    };
                    xulo_array_push(result, val, tag);
                }
            }
        }
        result
    }
}

/// 数组长度
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_array_len(arr: *const XuloArray) -> usize {
    unsafe {
        if arr.is_null() {
            0
        } else {
            (*arr).elements.len()
        }
    }
}

/// 数组获取
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_array_get(arr: *const XuloArray, index: usize) -> i64 {
    unsafe {
        if arr.is_null() || index >= (*arr).elements.len() {
            0
        } else {
            (&(*arr).elements)[index]
        }
    }
}

/// 数组设置（带类型标签）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_array_set(arr: *mut XuloArray, index: usize, value: i64, tag: i64) {
    unsafe {
        if !arr.is_null() && index < (*arr).elements.len() {
            let tag = if tag == -1 {
                xulo_detect_tag(value)
            } else {
                tag
            };
            (&mut (*arr).elements)[index] = value;
            if index < (*arr).tags.len() {
                (&mut (*arr).tags)[index] = tag;
            }
        }
    }
}

/// 对象获取字段
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_object_get(obj: *const XuloObject, field: *const c_char) -> i64 {
    unsafe {
        if obj.is_null() || field.is_null() {
            return 0;
        }

        let field_name = match CStr::from_ptr(field).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        };

        (*obj)
            .fields
            .iter()
            .find(|(name, _)| name == field_name)
            .map(|(_, value)| *value)
            .unwrap_or(0)
    }
}

/// 对象字段的运行时标签。缺失字段与取值缺省（0）配对成 null 标签，
/// 与 `Value::Null` 的渲染一致。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_object_get_tag(obj: *const XuloObject, field: *const c_char) -> i64 {
    unsafe {
        if obj.is_null() || field.is_null() {
            return 6;
        }

        let field_name = match CStr::from_ptr(field).to_str() {
            Ok(s) => s,
            Err(_) => return 6,
        };

        let obj = &*obj;
        obj.fields
            .iter()
            .position(|(name, _)| name == field_name)
            .and_then(|i| obj.field_tags.get(i).copied())
            .unwrap_or(6)
    }
}

/// 对象设置字段（带类型标签）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_object_set(
    obj: *mut XuloObject,
    field: *const c_char,
    value: i64,
    tag: i64,
) {
    unsafe {
        if obj.is_null() || field.is_null() {
            return;
        }

        let field_name = match CStr::from_ptr(field).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return,
        };

        let tag = if tag == -1 {
            xulo_detect_tag(value)
        } else {
            tag
        };
        let obj_mut = &mut *obj;
        if let Some(idx) = obj_mut
            .fields
            .iter()
            .position(|(name, _)| name == &field_name)
        {
            obj_mut.fields[idx].1 = value;
            if idx < obj_mut.field_tags.len() {
                obj_mut.field_tags[idx] = tag;
            }
        } else {
            obj_mut.fields.push((field_name, value));
            obj_mut.field_tags.push(tag);
        }
    }
}

/// 把一个 Rust 字符串驻留为运行时字符串（返回可交给 JIT 的 C 指针）
fn intern_string(s: impl Into<String>) -> *const c_char {
    let c_str = CString::new(s.into()).unwrap();
    let ptr = c_str.as_ptr();
    STRINGS.with(|strings| {
        strings.borrow_mut().push(c_str);
    });
    ptr
}

/// `std::fs` 的 AOT 落地：`Result` 枚举的 Ok/Err 构造与解释器一致（Err 携带
/// `Error(message)` 对象，字段 `message` 是字符串）。
fn fs_ok(value: i64, tag: i64) -> *mut XuloEnum {
    let e = xulo_alloc_enum(c"Result".as_ptr(), c"Ok".as_ptr(), 0, 1);
    xulo_enum_set(e, 0, value, tag);
    e
}

fn fs_err(message: String) -> *mut XuloEnum {
    let err_obj = xulo_alloc_object(1);
    xulo_object_set(
        err_obj,
        c"message".as_ptr(),
        intern_string(message) as i64,
        0,
    );
    let e = xulo_alloc_enum(c"Result".as_ptr(), c"Err".as_ptr(), 1, 1);
    xulo_enum_set(e, 0, err_obj as i64, 4);
    e
}

/// `std::fs::read(path)` -> `Result<string, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_read(path: *const c_char) -> *mut XuloEnum {
    let path = if path.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() }
    };
    match xulo_std::fs::read(&path) {
        Ok(text) => fs_ok(intern_string(text) as i64, 0),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::write(path, content)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_write(path: *const c_char, content: *const c_char) -> *mut XuloEnum {
    let path = if path.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() }
    };
    let content = if content.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(content).to_str().unwrap_or("").to_string() }
    };
    match xulo_std::fs::write(&path, &content) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::exists(path)` -> bool（总谓词，不失败）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_exists(path: *const c_char) -> i64 {
    let path = if path.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() }
    };
    i64::from(xulo_std::fs::exists(&path))
}

/// `std::fs::remove(path)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_remove(path: *const c_char) -> *mut XuloEnum {
    let path = if path.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() }
    };
    match xulo_std::fs::remove(&path) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// `std::fs::mkdir(path)` -> `Result<null, Error>`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_fs_mkdir(path: *const c_char) -> *mut XuloEnum {
    let path = if path.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(path).to_str().unwrap_or("").to_string() }
    };
    match xulo_std::fs::mkdir(&path) {
        Ok(()) => fs_ok(0, 6),
        Err(message) => fs_err(message),
    }
}

/// 一次正则匹配结果：普通对象（`value` 字段）+ 旁路登记的捕获组
fn regex_match_object(m: &xulo_std::regex::Match) -> *mut XuloObject {
    let value_ptr = intern_string(m.value.clone());
    let obj = xulo_alloc_object(1);
    xulo_object_set(obj, c"value".as_ptr(), value_ptr as i64, 0);
    MATCH_GROUPS.with(|groups| {
        groups.borrow_mut().insert(obj as usize, m.groups.clone());
    });
    obj
}

/// `RegExp::compile(pattern, flags)`：成功返回 `Result::Ok`（payload 为
/// 编译正则指针，tag=9），失败返回 `Result::Err`（payload 为带 `message`
/// 字段的 RegexError 对象）——非法模式与未知标志都是数据
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_regex_compile(
    pattern: *const c_char,
    flags: *const c_char,
) -> *mut XuloEnum {
    unsafe {
        let pattern = if pattern.is_null() {
            String::new()
        } else {
            CStr::from_ptr(pattern).to_str().unwrap_or("").to_string()
        };
        let flags = if flags.is_null() {
            String::new()
        } else {
            CStr::from_ptr(flags).to_str().unwrap_or("").to_string()
        };
        match xulo_std::regex::compile(&pattern, &flags) {
            Ok(re) => {
                let ptr = Box::into_raw(Box::new(re));
                REGEXPS.with(|rs| rs.borrow_mut().push(ptr));
                let e = xulo_alloc_enum(c"Result".as_ptr(), c"Ok".as_ptr(), 0, 1);
                xulo_enum_set(e, 0, ptr as i64, 9);
                e
            }
            Err(message) => {
                let msg_ptr = intern_string(message);
                let err_obj = xulo_alloc_object(1);
                xulo_object_set(err_obj, c"message".as_ptr(), msg_ptr as i64, 0);
                let e = xulo_alloc_enum(c"Result".as_ptr(), c"Err".as_ptr(), 1, 1);
                xulo_enum_set(e, 0, err_obj as i64, 4);
                e
            }
        }
    }
}

/// `re.test(input)` -> bool
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_regex_test(re: i64, input: *const c_char) -> i64 {
    unsafe {
        if re == 0 || input.is_null() {
            return 0;
        }
        let re = &*(re as *const xulo_std::regex::RegExp);
        let input = CStr::from_ptr(input).to_str().unwrap_or("");
        i64::from(re.test(input))
    }
}

/// `re.find(input)` -> 匹配对象或 null
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_regex_find(re: i64, input: *const c_char) -> i64 {
    unsafe {
        if re == 0 || input.is_null() {
            return 0;
        }
        let re = &*(re as *const xulo_std::regex::RegExp);
        let input = CStr::from_ptr(input).to_str().unwrap_or("");
        match re.find(input) {
            Some(m) => regex_match_object(&m) as i64,
            None => 0,
        }
    }
}

/// `re.find_all(input)` -> 匹配对象列表
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_regex_find_all(re: i64, input: *const c_char) -> i64 {
    unsafe {
        if re == 0 || input.is_null() {
            return 0;
        }
        let re = &*(re as *const xulo_std::regex::RegExp);
        let input = CStr::from_ptr(input).to_str().unwrap_or("");
        let ms = re.find_all(input);
        let arr = xulo_alloc_array(ms.len());
        for m in &ms {
            let obj = regex_match_object(m);
            xulo_array_push(arr, obj as i64, 4);
        }
        arr as i64
    }
}

/// `m.group(i)`：i 从 1 起（组 0 不对外，`value` 已携带整个匹配）；
/// 未参与或越界返回 null
#[unsafe(no_mangle)]
pub extern "C" fn xulo_regex_match_group(m: i64, index: i64) -> *const c_char {
    if m == 0 || index < 1 {
        return std::ptr::null();
    }
    let groups = MATCH_GROUPS.with(|g| g.borrow().get(&(m as usize)).cloned());
    let Some(groups) = groups else {
        return std::ptr::null();
    };
    match groups.get((index - 1) as usize).and_then(|g| g.as_ref()) {
        Some(s) => intern_string(s.clone()),
        None => std::ptr::null(),
    }
}

/// `"input".replace(re, "$2.$1")`：模板展开捕获组
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_regex_replace(
    re: i64,
    input: *const c_char,
    template: *const c_char,
) -> *const c_char {
    unsafe {
        if re == 0 || input.is_null() {
            return input;
        }
        let re = &*(re as *const xulo_std::regex::RegExp);
        let input = CStr::from_ptr(input).to_str().unwrap_or("");
        let template = if template.is_null() {
            ""
        } else {
            CStr::from_ptr(template).to_str().unwrap_or("")
        };
        intern_string(re.replace(input, template))
    }
}

/// `"input".replace(re, fn(m) { ... })`：替换函数是 JIT 闭包
/// （入口签名固定 `(env, a0..a7) -> i64`，返回字符串指针）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_regex_replace_with(
    re: i64,
    input: *const c_char,
    closure: i64,
) -> *const c_char {
    type ClosureEntry = extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64;
    unsafe {
        if re == 0 || input.is_null() {
            return input;
        }
        let re = &*(re as *const xulo_std::regex::RegExp);
        let input = CStr::from_ptr(input).to_str().unwrap_or("").to_string();
        let cl = closure as *const XuloClosure;
        if cl.is_null() || (*cl).func.is_null() {
            // 防御：语义层已校验可调用
            return std::ptr::null();
        }
        let func: ClosureEntry = std::mem::transmute((*cl).func);
        let env = (*cl).env as i64;
        let out = re
            .replace_with(&input, |m| {
                let obj = regex_match_object(m);
                let ret = func(env, obj as i64, 0, 0, 0, 0, 0, 0, 0);
                if ret == 0 {
                    Ok::<String, ()>(String::new())
                } else {
                    Ok::<String, ()>(
                        CStr::from_ptr(ret as *const c_char)
                            .to_str()
                            .unwrap_or("")
                            .to_string(),
                    )
                }
            })
            .unwrap_or_default();
        intern_string(out)
    }
}

/// `"abc".to_uppercase()`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_string_to_uppercase(s: *const c_char) -> *const c_char {
    unsafe {
        if s.is_null() {
            return intern_string(String::new());
        }
        let s = CStr::from_ptr(s).to_str().unwrap_or("");
        intern_string(s.to_uppercase())
    }
}

/// 清理所有分配的内存
#[unsafe(no_mangle)]
pub extern "C" fn xulo_cleanup() {
    OBJECTS.with(|objects| {
        for ptr in objects.borrow_mut().drain(..) {
            unsafe {
                let _ = Box::from_raw(ptr);
            }
        }
    });
    ARRAYS.with(|arrays| {
        for ptr in arrays.borrow_mut().drain(..) {
            unsafe {
                let _ = Box::from_raw(ptr);
            }
        }
    });

    ENUMS.with(|enums| {
        for ptr in enums.borrow_mut().drain(..) {
            unsafe {
                let _ = Box::from_raw(ptr);
            }
        }
    });

    CLOSURES.with(|closures| {
        for ptr in closures.borrow_mut().drain(..) {
            unsafe {
                let _ = Box::from_raw(ptr);
            }
        }
    });
    CLOSURE_PTRS.with(|ps| ps.borrow_mut().clear());

    REGEXPS.with(|rs| {
        for ptr in rs.borrow_mut().drain(..) {
            unsafe {
                let _ = Box::from_raw(ptr);
            }
        }
    });
    MATCH_GROUPS.with(|groups| groups.borrow_mut().clear());

    ENVS.with(|envs| {
        for ptr in envs.borrow_mut().drain(..) {
            unsafe {
                let _ = Box::from_raw(ptr);
            }
        }
    });

    CELLS.with(|cells| {
        for ptr in cells.borrow_mut().drain(..) {
            unsafe {
                let _ = Box::from_raw(ptr);
            }
        }
    });

    STRINGS.with(|strings| {
        strings.borrow_mut().clear();
    });

    // std::math::toFixed 交给编译产物的字符串由 xulo-std 持有，一并释放。
    xulo_std::ffi::release_strings();
}
