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
    unsafe { (cell as *mut XuloCell).as_ref().map(|c| c.value).unwrap_or(0) }
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
