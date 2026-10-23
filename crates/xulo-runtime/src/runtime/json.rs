use super::*;
use crate::value::Value;
use serde_json::Value as Json;
use std::rc::Rc;

// `std::json` 的运行时接线：serde_json 树与两套 Xulo 表示之间的转换——
// 解释器的 [`Value`]（`parse`/`stringify` 的 native 端），以及编译侧带
// tag 的 (载荷, 标签) 对（FFI 端）。`Result` 枚举沿用 `fs_ok`/`fs_err`。
// 对象键经 serde_json 的默认 Map（BTreeMap）按字典序落定；解析与序列化
// 都以 `xulo_std::json::MAX_DEPTH` 为深度上限，Xulo 列表/对象的自引用
// （环）在转换途中就会被上限拦成干净 Err。

/// 深度超限（与 `xulo_std::json` 共用同一上限与消息）。
fn depth_err() -> String {
    xulo_std::json::depth_err()
}

/// 非有限浮点（NaN / ±Infinity）没有 JSON 表示——`Number::from_f64`
/// 返回 `None` 时给出的消息。
fn nonfinite_err(n: f64) -> String {
    let label = if n.is_nan() {
        "NaN"
    } else if n > 0.0 {
        "Infinity"
    } else {
        "-Infinity"
    };
    format!("cannot serialize {label} as JSON")
}

/// f64 → JSON 数字：NaN/Infinity 没有 JSON 表示，是 Err。整数值浮点的
/// 归一（`2.0` → `2`）统一由 [`xulo_std::json::stringify`] 在落字时做。
fn json_number(n: f64) -> Result<Json, String> {
    serde_json::Number::from_f64(n)
        .map(Json::Number)
        .ok_or_else(|| nonfinite_err(n))
}

/// 解释器路径：`std::json::parse` 产出的 serde 树 → 运行时 `Value`。
/// serde 树无环且深度已被解析上限约束，这里不再查深度。
pub fn json_to_value(json: &Json) -> Value {
    match json {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        Json::String(s) => Value::String(s.as_str().into()),
        Json::Array(items) => {
            let list = items.iter().map(json_to_value).collect();
            Value::List(Rc::new(RefCell::new(list)))
        }
        Json::Object(map) => {
            let fields = map
                .iter()
                .map(|(k, v)| (k.clone(), json_to_value(v)))
                .collect();
            Value::Object(Rc::new(RefCell::new(fields)))
        }
    }
}

/// 解释器路径：`std::json::stringify` 的入参 `Value` → serde 树。
/// 环（自引用列表/对象）会在深度上限处变成 Err，而不是无界递归。
pub fn value_to_json(value: &Value, depth: usize) -> Result<Json, String> {
    if depth > xulo_std::json::MAX_DEPTH {
        return Err(depth_err());
    }
    match value {
        Value::Null => Ok(Json::Null),
        Value::Boolean(b) => Ok(Json::Bool(*b)),
        Value::Number(n) => json_number(*n),
        Value::String(s) => Ok(Json::String(s.to_string())),
        Value::List(items) => {
            let items = items.borrow();
            let mut out = Vec::with_capacity(items.len());
            for item in items.iter() {
                out.push(value_to_json(item, depth + 1)?);
            }
            Ok(Json::Array(out))
        }
        Value::Object(fields) => {
            let fields = fields.borrow();
            let mut map = serde_json::Map::new();
            for (k, v) in fields.iter() {
                map.insert(k.clone(), value_to_json(v, depth + 1)?);
            }
            Ok(Json::Object(map))
        }
        other => Err(format!("cannot serialize {} as JSON", other.kind_name())),
    }
}

/// 编译路径：serde 树 → (载荷位模式, 标签)。整数保持 tag=1 精确往返，
/// 其余数字（浮点、超出 i64 的 u64）走 tag=2 的 f64 位模式。递归深度
/// 受 serde_json 的解析上限约束（≤ [`xulo_std::json::MAX_DEPTH`]）。
fn json_to_payload(json: &Json, depth: usize) -> (i64, i64) {
    if depth > xulo_std::json::MAX_DEPTH {
        return (0, 6);
    }
    match json {
        Json::Null => (0, 6),
        Json::Bool(b) => (i64::from(*b), 5),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                (i, 1)
            } else {
                let bits = n.as_f64().unwrap_or(0.0).to_bits() as i64;
                xulo_register_float(bits);
                (bits, 2)
            }
        }
        Json::String(s) => (intern_string(s.as_str()) as i64, 0),
        Json::Array(items) => {
            let arr = xulo_alloc_array(items.len());
            for item in items {
                let (payload, tag) = json_to_payload(item, depth + 1);
                xulo_array_push(arr, payload, tag);
            }
            (arr as i64, 3)
        }
        Json::Object(map) => {
            let obj = xulo_alloc_object(map.len());
            for (k, v) in map {
                let (payload, tag) = json_to_payload(v, depth + 1);
                xulo_object_set(obj, intern_string(k.as_str()), payload, tag);
            }
            (obj as i64, 4)
        }
    }
}

/// 编译路径：(载荷位模式, 标签) → serde 树，供 `stringify` 消费。
/// tag 语义与 `xulo_print_value` 一致；环（自引用结构）在深度上限处
/// 变成 Err。
fn payload_to_json(value: i64, tag: i64, depth: usize) -> Result<Json, String> {
    if depth > xulo_std::json::MAX_DEPTH {
        return Err(depth_err());
    }
    match tag {
        0 => {
            let ptr = value as *const c_char;
            if ptr.is_null() {
                return Ok(Json::Null);
            }
            let text = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap_or("");
            Ok(Json::String(text.to_string()))
        }
        1 => Ok(Json::Number(value.into())),
        2 => json_number(f64::from_bits(value as u64)),
        3 => {
            let arr = value as *const XuloArray;
            if arr.is_null() {
                return Ok(Json::Array(Vec::new()));
            }
            let arr = unsafe { &*arr };
            let mut out = Vec::with_capacity(arr.elements.len());
            for (i, payload) in arr.elements.iter().enumerate() {
                let t = arr.tags.get(i).copied().unwrap_or(1);
                out.push(payload_to_json(*payload, t, depth + 1)?);
            }
            Ok(Json::Array(out))
        }
        4 => {
            let obj = value as *const XuloObject;
            if obj.is_null() {
                return Ok(Json::Object(serde_json::Map::new()));
            }
            let obj = unsafe { &*obj };
            let mut map = serde_json::Map::new();
            for (i, (key, payload)) in obj.fields.iter().enumerate() {
                let t = obj.field_tags.get(i).copied().unwrap_or(0);
                map.insert(key.clone(), payload_to_json(*payload, t, depth + 1)?);
            }
            Ok(Json::Object(map))
        }
        5 => Ok(Json::Bool(value != 0)),
        6 => Ok(Json::Null),
        7 => Err("cannot serialize an enum as JSON".to_string()),
        8 => Err("cannot serialize a function as JSON".to_string()),
        other => Err(format!("cannot serialize value with unknown tag {other}")),
    }
}

/// `std::json::parse(text)` -> `Result<any, Error>`（载荷 tag 由 JSON 值
/// 的种类决定：对象 4 / 列表 3 / 字符串 0 / 整数 1 / 浮点 2 / 布尔 5 /
/// null 6）
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_json_parse(text: *const c_char) -> *mut XuloEnum {
    let text = if text.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(text) }
            .to_str()
            .unwrap_or("")
            .to_string()
    };
    match xulo_std::json::parse(&text) {
        Ok(json) => {
            let (payload, tag) = json_to_payload(&json, 0);
            fs_ok(payload, tag)
        }
        Err(message) => fs_err(message),
    }
}

/// `std::json::stringify(value, tag)` -> `Result<string, Error>`。第二参
/// 是调用点按 `print` 同源规则推断出的运行时标签（编译侧在调用时追加，
/// 解释器不走此入口）。
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_json_stringify(value: i64, tag: i64) -> *mut XuloEnum {
    match payload_to_json(value, tag, 0).and_then(|json| xulo_std::json::stringify(&json)) {
        Ok(text) => fs_ok(intern_string(text) as i64, 0),
        Err(message) => fs_err(message),
    }
}
