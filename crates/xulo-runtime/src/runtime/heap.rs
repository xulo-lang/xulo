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
    OBJECT_PTRS.with(|ptrs| {
        ptrs.borrow_mut().insert(ptr as usize);
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
    ARRAY_PTRS.with(|ptrs| {
        ptrs.borrow_mut().insert(ptr as usize);
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
    ENUM_PTRS.with(|ptrs| {
        ptrs.borrow_mut().insert(ptr as usize);
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
    xulo_mark_value_array(arr as i64);
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

/// 对象展开：把 `src` 的字段按顺序写入 `dst`，同名键覆盖原值。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_object_extend(dst: *mut XuloObject, src: *const XuloObject) {
    if dst.is_null() || src.is_null() {
        return;
    }
    unsafe {
        let source = &*src;
        let target = &mut *dst;
        for (index, (name, value)) in source.fields.iter().enumerate() {
            let tag = source.field_tags.get(index).copied().unwrap_or(1);
            if let Some(slot) = target.fields.iter().position(|(key, _)| key == name) {
                target.fields[slot].1 = *value;
                if slot < target.field_tags.len() {
                    target.field_tags[slot] = tag;
                } else {
                    target.field_tags.resize(slot + 1, 1);
                    target.field_tags[slot] = tag;
                }
            } else {
                target.fields.push((name.clone(), *value));
                target.field_tags.push(tag);
            }
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
    SETS.with(|sets| sets.borrow_mut().insert(set as usize));
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

/// Structural `==` for tagged values (spec `expressions/operators.md`,
/// "Equality"): lists/arrays/tuples compare element-wise, objects (structs and
/// maps) compare by key, enums compare type, variant, and every payload value.
/// Sets are unordered. A visited-pair guard makes cyclic values terminate.
const MAX_VALUE_EQUAL_DEPTH: usize = 4096;

fn runtime_value_equal(
    left: i64,
    left_tag: i64,
    right: i64,
    right_tag: i64,
    seen: &mut Vec<(usize, usize)>,
    depth: usize,
) -> bool {
    if depth > MAX_VALUE_EQUAL_DEPTH {
        return false;
    }
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
    match (left_tag, right_tag) {
        (3, 3) => unsafe {
            if left == 0 || right == 0 {
                return left == right;
            }
            let lhs = &*(left as *const XuloArray);
            let rhs = &*(right as *const XuloArray);
            if lhs.elements.len() != rhs.elements.len() {
                return false;
            }
            let pair = (left as usize, right as usize);
            if seen.contains(&pair) {
                return true;
            }
            let lhs_is_set = SETS.with(|sets| sets.borrow().contains(&pair.0));
            let rhs_is_set = SETS.with(|sets| sets.borrow().contains(&pair.1));
            seen.push(pair);
            let result = if lhs_is_set || rhs_is_set {
                lhs_is_set
                    && rhs_is_set
                    && lhs.elements.iter().enumerate().all(|(index, value)| {
                        let tag = set_tag(&lhs.tags, index);
                        rhs.elements.iter().enumerate().any(|(other, candidate)| {
                            runtime_value_equal(
                                *value,
                                tag,
                                *candidate,
                                set_tag(&rhs.tags, other),
                                seen,
                                depth + 1,
                            )
                        })
                    })
            } else {
                lhs.elements.iter().enumerate().all(|(index, value)| {
                    runtime_value_equal(
                        *value,
                        set_tag(&lhs.tags, index),
                        rhs.elements[index],
                        set_tag(&rhs.tags, index),
                        seen,
                        depth + 1,
                    )
                })
            };
            seen.pop();
            result
        },
        (4, 4) => unsafe {
            if left == 0 || right == 0 {
                return left == right;
            }
            let lhs = &*(left as *const XuloObject);
            let rhs = &*(right as *const XuloObject);
            if lhs.fields.len() != rhs.fields.len() {
                return false;
            }
            let pair = (left as usize, right as usize);
            if seen.contains(&pair) {
                return true;
            }
            seen.push(pair);
            let result = lhs.fields.iter().enumerate().all(|(index, (key, value))| {
                let tag = lhs.field_tags.get(index).copied().unwrap_or(1);
                rhs.fields
                    .iter()
                    .position(|(other, _)| other == key)
                    .and_then(|other| {
                        rhs.field_tags
                            .get(other)
                            .copied()
                            .map(|other_tag| (other, other_tag))
                    })
                    .is_some_and(|(other, other_tag)| {
                        runtime_value_equal(
                            *value,
                            tag,
                            rhs.fields[other].1,
                            other_tag,
                            seen,
                            depth + 1,
                        )
                    })
            });
            seen.pop();
            result
        },
        (7, 7) => unsafe {
            if left == 0 || right == 0 {
                return left == right;
            }
            let lhs = &*(left as *const XuloEnum);
            let rhs = &*(right as *const XuloEnum);
            if lhs.variant != rhs.variant || lhs.elements.len() != rhs.elements.len() {
                return false;
            }
            let same_name = match (lhs.type_name.is_null(), rhs.type_name.is_null()) {
                (true, true) => true,
                (false, false) => {
                    CStr::from_ptr(lhs.type_name).to_bytes()
                        == CStr::from_ptr(rhs.type_name).to_bytes()
                }
                _ => false,
            };
            if !same_name {
                return false;
            }
            let pair = (left as usize, right as usize);
            if seen.contains(&pair) {
                return true;
            }
            seen.push(pair);
            let result = lhs.elements.iter().enumerate().all(|(index, value)| {
                runtime_value_equal(
                    *value,
                    lhs.tags.get(index).copied().unwrap_or(1),
                    rhs.elements[index],
                    rhs.tags.get(index).copied().unwrap_or(1),
                    seen,
                    depth + 1,
                )
            });
            seen.pop();
            result
        },
        _ => set_value_eq(left, left_tag, right, right_tag),
    }
}

/// `==` on two tagged runtime values; returns `1` for equal.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_value_equal(left: i64, left_tag: i64, right: i64, right_tag: i64) -> i64 {
    i64::from(runtime_value_equal(
        left,
        left_tag,
        right,
        right_tag,
        &mut Vec::new(),
        0,
    ))
}

/// `copy e` (spec `memory-and-runtime.md`, "Ownership with `copy`"): duplicate
/// every reachable list/array/tuple, object/struct/map, enum payload, and set,
/// so later writes through either copy stay invisible to the other. Strings are
/// immutable and identity-carrying values (functions, tasks, views, errors) are
/// not duplicated. A visited map keeps cyclic values finite, and the depth
/// guard bounds adversarial nesting.
const MAX_VALUE_COPY_DEPTH: usize = 4096;

fn value_copy_too_deep() -> ! {
    let message = intern_string("`copy` exceeded the maximum value depth");
    xulo_panic(message as i64, 0);
    unreachable!("xulo_panic exits the process")
}

fn runtime_value_copy(
    value: i64,
    tag: i64,
    seen: &mut std::collections::HashMap<usize, i64>,
    depth: usize,
) -> i64 {
    if depth > MAX_VALUE_COPY_DEPTH {
        value_copy_too_deep();
    }
    let tag = if tag == -1 {
        xulo_detect_tag(value)
    } else {
        tag
    };
    match tag {
        3 => unsafe {
            if value == 0 {
                return 0;
            }
            let key = value as usize;
            if let Some(copy) = seen.get(&key) {
                return *copy;
            }
            let source = &*(value as *const XuloArray);
            let copy = xulo_alloc_array(source.elements.len());
            if SETS.with(|sets| sets.borrow().contains(&key)) {
                SETS.with(|sets| sets.borrow_mut().insert(copy as usize));
            }
            seen.insert(key, copy as i64);
            for (index, element) in source.elements.iter().copied().enumerate() {
                let element_tag = set_tag(&source.tags, index);
                let copied = runtime_value_copy(element, element_tag, seen, depth + 1);
                xulo_array_push(copy, copied, element_tag);
            }
            copy as i64
        },
        4 => unsafe {
            if value == 0 {
                return 0;
            }
            let key = value as usize;
            if let Some(copy) = seen.get(&key) {
                return *copy;
            }
            let source = &*(value as *const XuloObject);
            let copy = xulo_alloc_object(source.fields.len());
            seen.insert(key, copy as i64);
            for (index, (name, field)) in source.fields.iter().enumerate() {
                let field_tag = source.field_tags.get(index).copied().unwrap_or(1);
                let copied = runtime_value_copy(*field, field_tag, seen, depth + 1);
                (*copy).fields.push((name.clone(), copied));
                (*copy).field_tags.push(field_tag);
            }
            copy as i64
        },
        7 => unsafe {
            if value == 0 {
                return 0;
            }
            let key = value as usize;
            if let Some(copy) = seen.get(&key) {
                return *copy;
            }
            let source = &*(value as *const XuloEnum);
            let copy = xulo_alloc_enum(
                source.type_name,
                source.variant_name,
                source.variant,
                source.elements.len(),
            );
            seen.insert(key, copy as i64);
            for (index, element) in source.elements.iter().copied().enumerate() {
                let element_tag = source.tags.get(index).copied().unwrap_or(1);
                let copied = runtime_value_copy(element, element_tag, seen, depth + 1);
                xulo_enum_set(copy, index, copied, element_tag);
            }
            copy as i64
        },
        // Strings are immutable; scalars, identity-carrying values, and regex
        // handles keep their identity (spec "Equality and identity").
        _ => value,
    }
}

/// `copy e` on a tagged runtime value; returns the copied value in the same
/// slot and tag.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_value_copy(value: i64, tag: i64) -> i64 {
    runtime_value_copy(value, tag, &mut std::collections::HashMap::new(), 0)
}

/// Mark a struct instance (spec `memory-and-runtime.md`, value semantics).
/// The `XuloObject` representation is shared with `Map`, so construction sites
/// register the value-semantic ones and `Map` stays a shared reference.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_mark_struct(value: i64) {
    if value != 0 {
        STRUCT_OBJECTS.with(|objects| {
            objects.borrow_mut().insert(value as usize);
        });
    }
}

/// Mark an `Array`/tuple value. `List`/`Set` share the `XuloArray`
/// representation but keep reference semantics, so only these creation sites
/// register.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_mark_value_array(value: i64) {
    if value != 0 {
        VALUE_ARRAYS.with(|arrays| {
            arrays.borrow_mut().insert(value as usize);
        });
    }
}

fn runtime_value_shallow_copy(
    value: i64,
    tag: i64,
    seen: &mut std::collections::HashMap<usize, i64>,
    depth: usize,
) -> i64 {
    if depth > MAX_VALUE_COPY_DEPTH {
        value_copy_too_deep();
    }
    let tag = if tag == -1 {
        xulo_detect_tag(value)
    } else {
        tag
    };
    match tag {
        3 => unsafe {
            if value == 0 {
                return 0;
            }
            let key = value as usize;
            let is_value = VALUE_ARRAYS.with(|arrays| arrays.borrow().contains(&key));
            if !is_value {
                // `List`/`Set`: reference-category, the binding shares it.
                return value;
            }
            if let Some(copy) = seen.get(&key) {
                return *copy;
            }
            let source = &*(value as *const XuloArray);
            let copy = xulo_alloc_array(source.elements.len());
            VALUE_ARRAYS.with(|arrays| {
                arrays.borrow_mut().insert(copy as usize);
            });
            seen.insert(key, copy as i64);
            for (index, element) in source.elements.iter().copied().enumerate() {
                let element_tag = set_tag(&source.tags, index);
                let copied = runtime_value_shallow_copy(element, element_tag, seen, depth + 1);
                xulo_array_push(copy, copied, element_tag);
            }
            copy as i64
        },
        4 => unsafe {
            if value == 0 {
                return 0;
            }
            let key = value as usize;
            let is_value = STRUCT_OBJECTS.with(|objects| objects.borrow().contains(&key));
            if !is_value {
                // `Map`: reference-category, the binding shares it.
                return value;
            }
            if let Some(copy) = seen.get(&key) {
                return *copy;
            }
            let source = &*(value as *const XuloObject);
            let copy = xulo_alloc_object(source.fields.len());
            STRUCT_OBJECTS.with(|objects| {
                objects.borrow_mut().insert(copy as usize);
            });
            seen.insert(key, copy as i64);
            for (index, (name, field)) in source.fields.iter().enumerate() {
                let field_tag = source.field_tags.get(index).copied().unwrap_or(1);
                let copied = runtime_value_shallow_copy(*field, field_tag, seen, depth + 1);
                (*copy).fields.push((name.clone(), copied));
                (*copy).field_tags.push(field_tag);
            }
            copy as i64
        },
        // Enums are always value-category, so every binding gets its own
        // variant; the payload follows the same per-category rule.
        7 => unsafe {
            if value == 0 {
                return 0;
            }
            let key = value as usize;
            if let Some(copy) = seen.get(&key) {
                return *copy;
            }
            let source = &*(value as *const XuloEnum);
            let copy = xulo_alloc_enum(
                source.type_name,
                source.variant_name,
                source.variant,
                source.elements.len(),
            );
            seen.insert(key, copy as i64);
            for (index, element) in source.elements.iter().copied().enumerate() {
                let element_tag = source.tags.get(index).copied().unwrap_or(1);
                let copied = runtime_value_shallow_copy(element, element_tag, seen, depth + 1);
                xulo_enum_set(copy, index, copied, element_tag);
            }
            copy as i64
        },
        // Scalars, strings, functions, tasks, and other identity-carrying
        // values are already values for binding.
        _ => value,
    }
}

/// Binding/assignment copy (spec `memory-and-runtime.md`, "Value semantics and
/// reference semantics"): a value-category aggregate gets a fresh container
/// while reference-category values keep sharing. Unlike [`xulo_value_copy`],
/// which is the explicit deep copy of `copy e`, this shallow copy stops at the
/// first reference-category container.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_value_shallow_copy(value: i64, tag: i64) -> i64 {
    runtime_value_shallow_copy(value, tag, &mut std::collections::HashMap::new(), 0)
}

/// `Array::filled<T, N>(value)`: a fixed array of `N` copies of `value`.
/// The native representation is the list-shaped `XuloArray`; the static type
/// carries the length (spec `types/composite-types.md`).
#[unsafe(no_mangle)]
pub extern "C" fn xulo_array_filled(value: i64, tag: i64, len: i64) -> *mut XuloArray {
    let array = xulo_alloc_array(0);
    if len < 0 {
        let message = intern_string("array length must be non-negative");
        xulo_panic(message as i64, 0);
    }
    let tag = if tag == -1 {
        xulo_detect_tag(value)
    } else {
        tag
    };
    for _ in 0..len {
        xulo_array_push(array, value, tag);
    }
    xulo_mark_value_array(array as i64);
    array
}
