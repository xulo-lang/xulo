//! The `Json` namespace in isolation — serde_json-level `parse`/`stringify`
//! semantics with no Xulo pipeline involved: tree shapes, syntax errors,
//! round-trips, and the depth ceiling that keeps cyclic structures honest.
//! The Xulo-value conversions live in `xulo-runtime` (both representations:
//! the interpreter's `Value` and the compiled tagged payloads).

use xulo_std::json::{MAX_DEPTH, parse, stringify};

#[test]
fn parse_builds_the_tree_the_text_describes() {
    let json = parse(r#"{"a": 1, "b": [true, null, "x"], "c": 2.5}"#).expect("valid JSON");
    assert_eq!(json["a"], 1);
    assert_eq!(json["b"][0], true);
    assert!(json["b"][1].is_null());
    assert_eq!(json["b"][2], "x");
    assert_eq!(json["c"].as_f64(), Some(2.5));
}

#[test]
fn parse_accepts_every_json_scalar() {
    assert_eq!(parse("null").expect("null"), serde_json::Value::Null);
    assert_eq!(parse("true").expect("bool"), serde_json::Value::Bool(true));
    assert_eq!(parse("-7").expect("int"), serde_json::json!(-7));
    assert_eq!(parse("\"hi\"").expect("str"), serde_json::json!("hi"));
}

#[test]
fn parse_rejects_syntax_errors_with_serde_diagnostics() {
    let err = parse("{oops").expect_err("not JSON");
    assert!(err.starts_with("invalid JSON: "), "unexpected: {err}");
    let err = parse("").expect_err("empty input");
    assert!(err.starts_with("invalid JSON: "), "unexpected: {err}");
}

#[test]
fn stringify_round_trips_a_tree_to_compact_text() {
    let json = parse(r#"{"a": [1, "two", false, null], "b": 2.5}"#).expect("valid JSON");
    let text = stringify(&json).expect("serializable");
    assert_eq!(text, r#"{"a":[1,"two",false,null],"b":2.5}"#);
    assert_eq!(
        parse(&text).expect("reparse"),
        json,
        "round trip must be semantically identical"
    );
}

#[test]
fn stringify_normalizes_integral_floats_to_integers() {
    // `2.0` is integral: it serializes as `2`, matching how `print` renders
    // the same value and keeping integer round trips byte-stable.
    let json = serde_json::json!(2.0);
    assert_eq!(stringify(&json).expect("serializable"), "2");
    assert_eq!(
        stringify(&serde_json::json!(-0.0)).expect("serializable"),
        "0"
    );
    assert_eq!(
        stringify(&serde_json::json!(2.5)).expect("serializable"),
        "2.5"
    );
}

#[test]
fn stringify_refuses_trees_deeper_than_the_ceiling() {
    // Hand-build a ring-deep tree (parse's own recursion limit would stop
    // text at the same ceiling): the serializer must fail, not overflow.
    let mut deep = serde_json::json!(0);
    for _ in 0..(MAX_DEPTH + 8) {
        deep = serde_json::Value::Array(vec![deep]);
    }
    let err = stringify(&deep).expect_err("too deep");
    assert!(err.contains("nests deeper"), "unexpected: {err}");
}
