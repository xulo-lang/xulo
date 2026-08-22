use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

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
    static OBJECTS: RefCell<Vec<*mut XuloObject>> = RefCell::new(Vec::new());
    static ARRAYS: RefCell<Vec<*mut XuloArray>> = RefCell::new(Vec::new());
    static STRINGS: RefCell<Vec<CString>> = RefCell::new(Vec::new());
    static FLOATS: RefCell<Vec<i64>> = RefCell::new(Vec::new());
    /// JIT 嵌入的字符串指针（仅记录地址，不拥有所有权）
    static JIT_STRINGS: RefCell<Vec<*const c_char>> = RefCell::new(Vec::new());
    /// 输出缓冲区（用于测试捕获 print 输出）
    static OUTPUT: RefCell<Vec<String>> = RefCell::new(Vec::new());
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
        2 => {
            let bits = value as u64;
            let float_val = f64::from_bits(bits);
            if float_val.fract() == 0.0 && float_val.abs() < 1e15 {
                format!("{}", float_val as i64)
            } else {
                format!("{}", float_val)
            }
        }
        3 => {
            let arr = value as *const XuloArray;
            unsafe {
                if arr.is_null() {
                    "[]".to_string()
                } else {
                    let elements = &(*arr).elements;
                    let tags = &(*arr).tags;
                    let strs: Vec<String> = elements.iter().enumerate()
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
                    let strs: Vec<String> = fields.iter().enumerate()
                        .map(|(i, (k, v))| {
                            let t = if i < field_tags.len() { field_tags[i] } else { 0 };
                            format!("{}: {}", k, format_value(*v, t))
                        })
                        .collect();
                    format!("{{{}}}", strs.join(", "))
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
    let tag = if tag == -1 { xulo_detect_tag(value) } else { tag };
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
        strings.borrow().iter().any(|s| std::ptr::eq(s.as_ptr(), ptr))
    });
    if found { return 0; }
    
    // 检查是否是已知字符串（JIT 嵌入）
    found = JIT_STRINGS.with(|strings| {
        strings.borrow().iter().any(|s| std::ptr::eq(*s, ptr))
    });
    if found { return 0; }
    
    // 检查是否是已知数组
    found = ARRAYS.with(|arrays| {
        arrays.borrow().iter().any(|a| std::ptr::eq(*a as *const c_char, ptr))
    });
    if found { return 3; }
    
    // 检查是否是已知对象
    found = OBJECTS.with(|objects| {
        objects.borrow().iter().any(|o| std::ptr::eq(*o as *const c_char, ptr))
    });
    if found { return 4; }
    
    // 检查是否是已知浮点数
    found = FLOATS.with(|floats| {
        floats.borrow().iter().any(|f| *f == value)
    });
    if found { return 2; }
    
    // 默认为整数
    1
}

/// panic 函数
#[unsafe(no_mangle)]
pub extern "C" fn xulo_panic(ptr: *const c_char) {
    unsafe {
        if !ptr.is_null() {
            let c_str = CStr::from_ptr(ptr);
            if let Ok(s) = c_str.to_str() {
                eprintln!("panic: {}", s);
            }
        } else {
            eprintln!("panic: unknown error");
        }
        std::process::exit(1);
    }
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

/// 转换为字符串
#[unsafe(no_mangle)]
pub extern "C" fn xulo_to_string(value: i64) -> *const c_char {
    let result = if value == 0 {
        "null".to_string()
    } else {
        value.to_string()
    };
    
    let c_str = CString::new(result).unwrap();
    let ptr = c_str.as_ptr();
    
    STRINGS.with(|strings| {
        strings.borrow_mut().push(c_str);
    });
    
    ptr
}

/// 数组 push（带类型标签）
/// tag: 0=string, 1=int, 2=float, 3=array, 4=object, 5=bool, 6=null, -1=自动检测
#[unsafe(no_mangle)]
pub extern "C" fn xulo_array_push(arr: *mut XuloArray, value: i64, tag: i64) {
    unsafe {
        if !arr.is_null() {
            let tag = if tag == -1 { xulo_detect_tag(value) } else { tag };
            (*arr).elements.push(value);
            (*arr).tags.push(tag);
        }
    }
}

/// 数组获取标签
#[unsafe(no_mangle)]
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
pub extern "C" fn xulo_array_concat(a: *const XuloArray, b: *const XuloArray) -> *mut XuloArray {
    unsafe {
        let len_a = if a.is_null() { 0 } else { (*a).elements.len() };
        let len_b = if b.is_null() { 0 } else { (*b).elements.len() };
        let result = xulo_alloc_array(len_a + len_b);
        if !result.is_null() {
            if !a.is_null() {
                for i in 0..len_a {
                    let val = (&(*a).elements)[i];
                    let tag = if i < (*a).tags.len() { (&(*a).tags)[i] } else { 1 };
                    xulo_array_push(result, val, tag);
                }
            }
            if !b.is_null() {
                for i in 0..len_b {
                    let val = (&(*b).elements)[i];
                    let tag = if i < (*b).tags.len() { (&(*b).tags)[i] } else { 1 };
                    xulo_array_push(result, val, tag);
                }
            }
        }
        result
    }
}

/// 数组长度
#[unsafe(no_mangle)]
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
pub extern "C" fn xulo_array_set(arr: *mut XuloArray, index: usize, value: i64, tag: i64) {
    unsafe {
        if !arr.is_null() && index < (*arr).elements.len() {
            let tag = if tag == -1 { xulo_detect_tag(value) } else { tag };
            (&mut (*arr).elements)[index] = value;
            if index < (*arr).tags.len() {
                (&mut (*arr).tags)[index] = tag;
            }
        }
    }
}

/// 对象获取字段
#[unsafe(no_mangle)]
pub extern "C" fn xulo_object_get(obj: *const XuloObject, field: *const c_char) -> i64 {
    unsafe {
        if obj.is_null() || field.is_null() {
            return 0;
        }
        
        let field_name = match CStr::from_ptr(field).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        };
        
        (*obj).fields
            .iter()
            .find(|(name, _)| name == field_name)
            .map(|(_, value)| *value)
            .unwrap_or(0)
    }
}

/// 对象设置字段（带类型标签）
#[unsafe(no_mangle)]
pub extern "C" fn xulo_object_set(obj: *mut XuloObject, field: *const c_char, value: i64, tag: i64) {
    unsafe {
        if obj.is_null() || field.is_null() {
            return;
        }
        
        let field_name = match CStr::from_ptr(field).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return,
        };
        
        let tag = if tag == -1 { xulo_detect_tag(value) } else { tag };
        let obj_mut = &mut *obj;
        if let Some(idx) = obj_mut.fields.iter().position(|(name, _)| name == &field_name) {
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
