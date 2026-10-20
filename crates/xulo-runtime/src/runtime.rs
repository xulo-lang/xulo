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

thread_local! {
    static OBJECTS: RefCell<Vec<*mut XuloObject>> = const { RefCell::new(Vec::new()) };
    static ARRAYS: RefCell<Vec<*mut XuloArray>> = const { RefCell::new(Vec::new()) };
    static STRINGS: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };
    static FLOATS: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
    /// JIT 嵌入的字符串指针（仅记录地址，不拥有所有权）
    static JIT_STRINGS: RefCell<Vec<*const c_char>> = const { RefCell::new(Vec::new()) };
    /// 输出缓冲区（用于测试捕获 print 输出）
    static OUTPUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
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
        _ => format!("{}", value),
    }
}

/// 带类型标签的打印函数
/// tag: 0=string, 1=int, 2=float, 3=array, 4=object, 5=bool, 6=null, -1=自动检测
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
/// tag: 0=string, 1=int, 2=float, 3=array, 4=object, 5=bool, 6=null
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

/// Math 命名空间的 float 返回函数（对齐解释器 `native_math_*`，spec
/// `builtins/intrinsic-functions.md`）：参数与返回值均为 f64。
#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_abs(x: f64) -> f64 {
    x.abs()
}

/// `Math::min`：NaN 操作数传播（Rust `f64::min` 会把 NaN 当作"缺失"，须显式对齐解释器）。
#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

/// `Math::floor` / `Math::ceil` / `Math::round`（halfway away from zero）/
/// `Math::trunc`：返回仍是整数值的 float。
#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_floor(x: f64) -> f64 {
    x.floor()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_ceil(x: f64) -> f64 {
    x.ceil()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_round(x: f64) -> f64 {
    x.round()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_trunc(x: f64) -> f64 {
    x.trunc()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_sqrt(x: f64) -> f64 {
    x.sqrt()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_cbrt(x: f64) -> f64 {
    x.cbrt()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_pow(base: f64, exp: f64) -> f64 {
    base.powf(exp)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_sin(x: f64) -> f64 {
    x.sin()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_cos(x: f64) -> f64 {
    x.cos()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_tan(x: f64) -> f64 {
    x.tan()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_asin(x: f64) -> f64 {
    x.asin()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_acos(x: f64) -> f64 {
    x.acos()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_atan(x: f64) -> f64 {
    x.atan()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_atan2(y: f64, x: f64) -> f64 {
    y.atan2(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_log(x: f64) -> f64 {
    x.ln()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_log2(x: f64) -> f64 {
    x.log2()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_log10(x: f64) -> f64 {
    x.log10()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_exp(x: f64) -> f64 {
    x.exp()
}

/// `Math::toFixed(x, digits)`：与解释器 `native_math_to_fixed` 同一格式化路径。
/// `digits` 越界是运行时失败（spec `memory-and-runtime.md`），此处与解释器一样报错退出。
#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_to_fixed(x: f64, digits: i64) -> *const c_char {
    if !(0..=15).contains(&digits) {
        eprintln!("`Math::toFixed` digits must be an integer in 0..=15, found {digits}");
        std::process::exit(1);
    }
    let digits = digits as usize;
    let s = if x.is_nan() || x.is_infinite() {
        format_number(x)
    } else {
        let factor = 10f64.powi(digits as i32);
        let scaled = x * factor;
        if scaled.is_infinite() {
            format!("{x:.*}", digits)
        } else {
            format!("{:.*}", digits, scaled.round() / factor)
        }
    };
    let c_str = CString::new(s).unwrap();
    let ptr = c_str.as_ptr();
    STRINGS.with(|strings| {
        strings.borrow_mut().push(c_str);
    });
    ptr
}

/// Time 命名空间：`now` / `now_nanos` 为 epoch 值，`monotonic` 以进程首次调用为原点，
/// 与解释器一致（值返回 i64，math_time.xulo 只做比较，不打印裸时间戳）。
#[unsafe(no_mangle)]
pub extern "C" fn xulo_time_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_time_now_nanos() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

fn monotonic_start() -> std::time::Instant {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *START.get_or_init(std::time::Instant::now)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_time_monotonic() -> i64 {
    monotonic_start().elapsed().as_millis() as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_time_monotonic_nanos() -> i64 {
    monotonic_start().elapsed().as_nanos() as i64
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

    STRINGS.with(|strings| {
        strings.borrow_mut().clear();
    });
}
