//! JSON 文本与 `serde_json` 树之间的纯函数（`std::json`）。
//!
//! Xulo 运行时值 ↔ serde_json 树的双向转换住在 `xulo-runtime`
//! （解释器 `Value` 与编译侧带 tag 载荷是两套表示，都得转），本模块
//! 只碰 serde 类型，保持 `xulo-std` 对运行时值体系零依赖。

/// 序列化/反序列化的嵌套深度上限：serde_json 的解析递归限制与本模块
/// 的输出检查同用此值。Xulo 的列表/对象允许自引用（环），转换端也按
/// 同一上限拒绝，保证 `stringify` 不会无界递归。
pub const MAX_DEPTH: usize = 128;

/// 解析 JSON 文本为 serde_json 树；语法错误（含超出递归上限）是
/// `Err`，消息由 serde_json 给出。
pub fn parse(text: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))
}

/// 序列化 serde_json 树为 JSON 文本（紧凑单行）。树按
/// [`MAX_DEPTH`] 检查后落字：整数值的浮点（`2.0`）统一写成 `2`，与
/// `print` 的渲染一致、也让整数往返字节等价；serde_json 树是无环所有权
/// 树，深度检查过后序列化不会失败。
pub fn stringify(value: &serde_json::Value) -> Result<String, String> {
    let value = normalize(value, 0)?;
    Ok(value.to_string())
}

/// 深度检查 + 整数值浮点归一的单趟重写。
fn normalize(value: &serde_json::Value, depth: usize) -> Result<serde_json::Value, String> {
    if depth > MAX_DEPTH {
        return Err(depth_err());
    }
    Ok(match value {
        serde_json::Value::Number(n) => {
            if n.is_f64()
                && let Some(f) = n.as_f64()
                && f.fract() == 0.0
                && f.abs() <= 9_007_199_254_740_992.0
            {
                // 仅动浮点形态的整数：整数变体（含超出 2^53 的 u64）
                // 原样保留，避免经 f64 往返掉精度
                serde_json::Value::Number((f as i64).into())
            } else {
                value.clone()
            }
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|item| normalize(item, depth + 1))
                .collect::<Result<_, _>>()?,
        ),
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), normalize(v, depth + 1)?)))
                .collect::<Result<_, String>>()?,
        ),
        _ => value.clone(),
    })
}

/// 深度超限的统一错误消息（转换端与本模块共用）。
pub fn depth_err() -> String {
    format!("JSON value nests deeper than {MAX_DEPTH} levels")
}
