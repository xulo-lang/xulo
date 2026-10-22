use super::*;

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
