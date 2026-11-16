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
            (*arr).tags.push(tag as u8);
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
            (&(*arr).tags)[index] as i64
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
                let elems = &(*a).elements;
                (*result).elements.extend_from_slice(elems);
                (*result).tags.extend_from_slice(&(*a).tags);
                // 保持 tags 与 elements 对齐；缺失的标签按 int 处理。
                (*result).tags.resize(elems.len(), 1);
            }
            if !b.is_null() {
                let elems = &(*b).elements;
                let start = (*result).elements.len();
                (*result).elements.extend_from_slice(elems);
                (*result).tags.extend_from_slice(&(*b).tags);
                (*result).tags.resize(start + elems.len(), 1);
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
                (&mut (*arr).tags)[index] = tag as u8;
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

/// 对象/Map 是否含某键
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_object_has(obj: *const XuloObject, field: *const c_char) -> i64 {
    unsafe {
        if obj.is_null() || field.is_null() {
            return 0;
        }
        let field_name = match CStr::from_ptr(field).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        };
        i64::from((*obj).fields.iter().any(|(name, _)| name == field_name))
    }
}

/// 移除对象/Map 的一个键并返回原值；缺失时返回 0。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_object_remove(obj: *mut XuloObject, field: *const c_char) -> i64 {
    unsafe {
        if obj.is_null() || field.is_null() {
            return 0;
        }
        let field_name = match CStr::from_ptr(field).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        };
        let obj = &mut *obj;
        let Some(index) = obj.fields.iter().position(|(name, _)| name == field_name) else {
            return 0;
        };
        let (_, value) = obj.fields.remove(index);
        if index < obj.field_tags.len() {
            obj.field_tags.remove(index);
        }
        value
    }
}

/// 对象/Map 的条目数
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_object_size(obj: *const XuloObject) -> usize {
    unsafe {
        if obj.is_null() {
            0
        } else {
            (*obj).fields.len()
        }
    }
}

/// Map 的键列表（插入顺序）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_object_keys(obj: *const XuloObject) -> *mut XuloArray {
    let array = xulo_alloc_array(0);
    if obj.is_null() {
        return array;
    }
    unsafe {
        for (name, _) in &(*obj).fields {
            let key = intern_string(name.clone());
            xulo_array_push(array, key as i64, 0);
        }
    }
    array
}

/// Map 的值列表（插入顺序）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_object_values(obj: *const XuloObject) -> *mut XuloArray {
    let array = xulo_alloc_array(0);
    if obj.is_null() {
        return array;
    }
    unsafe {
        let object = &*obj;
        for (index, (_, value)) in object.fields.iter().enumerate() {
            let tag = object.field_tags.get(index).copied().unwrap_or(1);
            xulo_array_push(array, *value, tag);
        }
    }
    array
}

/// Map 的 `(key, value)` 对列表（插入顺序）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_object_entries(obj: *const XuloObject) -> *mut XuloArray {
    let array = xulo_alloc_array(0);
    if obj.is_null() {
        return array;
    }
    unsafe {
        let object = &*obj;
        for (index, (name, value)) in object.fields.iter().enumerate() {
            let tag = object.field_tags.get(index).copied().unwrap_or(1);
            let pair = xulo_alloc_array(0);
            let key = intern_string(name.clone());
            xulo_array_push(pair, key as i64, 0);
            xulo_array_push(pair, *value, tag);
            xulo_array_push(array, pair as i64, 3);
        }
    }
    array
}

/// `Set` 的元素标签；缺失槽位按 int 处理。
fn set_tag(tags: &[u8], index: usize) -> i64 {
    tags.get(index).copied().unwrap_or(1) as i64
}

/// `Set` 元素/Map 键的相等判定：数字按数值、字符串按内容、枚举按
/// 类型与变体；`NaN` 彼此相等（spec `types/composite-types.md`）。
fn set_value_eq(left: i64, left_tag: i64, right: i64, right_tag: i64) -> bool {
    let left_tag = if left_tag == -1 {
        xulo_detect_tag(left)
    } else {
        left_tag
    };
    let right_tag = if right_tag == -1 {
        xulo_detect_tag(right)
    } else {
        right_tag
    };
    let as_f64 = |value: i64, tag: i64| match tag {
        1 => Some(value as f64),
        2 => Some(f64::from_bits(value as u64)),
        10 => Some((value as u64) as f64),
        _ => None,
    };
    match (left_tag, right_tag) {
        (1, 1) => left == right,
        (10, 10) => left as u64 == right as u64,
        (1, 10) => left >= 0 && left as u64 == right as u64,
        (10, 1) => right >= 0 && right as u64 == left as u64,
        (2, 2) | (1, 2) | (2, 1) | (10, 2) | (2, 10) => {
            let (Some(left), Some(right)) = (as_f64(left, left_tag), as_f64(right, right_tag))
            else {
                return false;
            };
            (left.is_nan() && right.is_nan()) || left == right
        }
        (0, 0) => unsafe {
            if left == 0 || right == 0 {
                return left == right;
            }
            CStr::from_ptr(left as *const c_char).to_bytes()
                == CStr::from_ptr(right as *const c_char).to_bytes()
        },
        (5, 5) => (left != 0) == (right != 0),
        (6, 6) => true,
        (7, 7) => unsafe {
            if left == 0 || right == 0 {
                return left == right;
            }
            let left = &*(left as *const XuloEnum);
            let right = &*(right as *const XuloEnum);
            left.variant == right.variant
                && CStr::from_ptr(left.type_name).to_bytes()
                    == CStr::from_ptr(right.type_name).to_bytes()
        },
        _ => left == right,
    }
}

fn set_find(set: &XuloArray, value: i64, tag: i64) -> Option<usize> {
    set.elements
        .iter()
        .enumerate()
        .position(|(index, seen)| set_value_eq(*seen, set_tag(&set.tags, index), value, tag))
}

/// `Set::from_list(values)`：按列表顺序保留每个不同值的首次出现。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_set_from_list(values: *const XuloArray) -> *mut XuloArray {
    let set = xulo_alloc_array(0);
    if values.is_null() {
        return set;
    }
    unsafe {
        let values = &*values;
        for (index, value) in values.elements.iter().copied().enumerate() {
            let tag = set_tag(&values.tags, index);
            if set_find(&*set, value, tag).is_none() {
                xulo_array_push(set, value, tag);
            }
        }
    }
    set
}

/// `Set::has(value)`
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_set_has(set: *const XuloArray, value: i64, tag: i64) -> i64 {
    if set.is_null() {
        return 0;
    }
    i64::from(unsafe { set_find(&*set, value, tag).is_some() })
}

/// `Set::insert(value)`：返回是否真的插入。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_set_insert(set: *mut XuloArray, value: i64, tag: i64) -> i64 {
    if set.is_null() {
        return 0;
    }
    unsafe {
        if set_find(&*set, value, tag).is_some() {
            return 0;
        }
        xulo_array_push(set, value, tag);
    }
    1
}

/// `Set::remove(value)`：返回是否真的移除。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_set_remove(set: *mut XuloArray, value: i64, tag: i64) -> i64 {
    if set.is_null() {
        return 0;
    }
    unsafe {
        let Some(index) = set_find(&*set, value, tag) else {
            return 0;
        };
        (*set).elements.remove(index);
        if index < (*set).tags.len() {
            (*set).tags.remove(index);
        }
    }
    1
}

/// `Map::from_entries(entries)`：按列表顺序建 Map，重复键保留后一个值。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_map_from_entries(entries: *const XuloArray) -> *mut XuloObject {
    let object = xulo_alloc_object(0);
    if entries.is_null() {
        return object;
    }
    unsafe {
        let entries = &*entries;
        for raw_pair in entries.elements.iter() {
            let pair = *raw_pair as *const XuloArray;
            if pair.is_null() {
                continue;
            }
            let pair = &*pair;
            if pair.elements.len() < 2 {
                continue;
            }
            let key_tag = pair.tags.first().copied().unwrap_or(0);
            let key = intern_string(format_value(pair.elements[0], key_tag as i64));
            let value_tag = pair.tags.get(1).copied().unwrap_or(1);
            xulo_object_set(object, key, pair.elements[1], value_tag as i64);
        }
    }
    object
}
