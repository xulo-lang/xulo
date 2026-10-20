//! The `regex` module semantics in isolation — compilation, flags, matching,
//! and both replacement styles. No language involved.

use xulo_std::regex::{RegExp, compile};

#[test]
fn compile_rejects_an_illegal_pattern_with_the_parse_error() {
    let err = compile("([unclosed", "").expect_err("unbalanced group");
    assert!(
        err.contains("regex parse error") || err.contains("unclosed"),
        "unexpected message: {err}"
    );
}

#[test]
fn compile_rejects_an_unknown_flag() {
    let err = compile("abc", "ixz").expect_err("x is not a flag");
    assert_eq!(err, "unknown flag `x`");
    // Known flags still compile even when mixed with junk later — the first
    // unknown one wins, and `g`/`i`/`m`/`u` are all accepted.
    assert!(compile("abc", "igmu").is_ok());
    assert!(compile("abc", "").is_ok());
}

#[test]
fn test_is_case_sensitive_unless_the_i_flag_says_otherwise() {
    let sensitive = compile("^abc", "").unwrap();
    assert!(!sensitive.test("ABC"));
    let insensitive = compile("^abc", "i").unwrap();
    assert!(insensitive.test("ABC"));
    assert!(!insensitive.test("abd"));
}

#[test]
fn find_returns_the_first_match_with_value_span_and_groups() {
    let re = compile(r"(\d+)-(\d+)", "").unwrap();
    let m = re.find("ab 12-34 cd 56-78").expect("a match");
    assert_eq!(m.value, "12-34");
    assert_eq!(m.group(1), Some("12"));
    assert_eq!(m.group(2), Some("34"));
    assert_eq!(m.group(3), None, "index out of range is absent");
    assert_eq!(m.start, 3);
    assert_eq!(m.end, 8);
}

#[test]
fn find_misses_report_none_and_absent_groups_report_none() {
    let re = compile(r"(a)?b", "").unwrap();
    assert!(re.find("zzz").is_none());

    let m = re.find("b").expect("b matches");
    assert_eq!(m.value, "b");
    assert_eq!(
        m.group(1),
        None,
        "a group that did not participate is absent"
    );
}

#[test]
fn find_all_yields_every_match_in_order_regardless_of_g() {
    let without_g = compile(r"\d+", "").unwrap();
    let with_g = compile(r"\d+", "g").unwrap();
    let collect = |re: &RegExp| {
        re.find_all("a1 b22 c333")
            .iter()
            .map(|m| m.value.clone())
            .collect::<Vec<_>>()
    };
    let expected = vec!["1".to_string(), "22".to_string(), "333".to_string()];
    assert_eq!(collect(&without_g), expected);
    assert_eq!(collect(&with_g), expected);
}

#[test]
fn replace_substitutes_only_the_first_match_without_g() {
    let re = compile(r"\d+", "").unwrap();
    assert_eq!(re.replace("a1 b2 c3", "N"), "aN b2 c3");
}

#[test]
fn replace_substitutes_every_match_with_g() {
    let re = compile(r"\d+", "g").unwrap();
    assert_eq!(re.replace("a1 b2 c3", "N"), "aN bN cN");
}

#[test]
fn replace_template_expands_numeric_and_named_groups() {
    let dated = compile(r"(\d{4})-(\d{2})-(\d{2})", "").unwrap();
    assert_eq!(
        dated.replace("on 2026-09-07 ok", "$2/$3/$1"),
        "on 09/07/2026 ok"
    );

    let named = compile(r"(?P<year>\d{4})-(?P<month>\d{2})", "").unwrap();
    assert_eq!(named.replace("2026-09", "$month/$year"), "09/2026");
}

#[test]
fn replace_with_callback_splices_replacements_between_surrounding_text() {
    let re = compile(r"[a-z]+", "g").unwrap();
    let out = re
        .replace_with("hi there", |m| Ok::<String, ()>(m.value.to_uppercase()))
        .unwrap();
    assert_eq!(out, "HI THERE");

    // Without `g` only the first match goes through the callback.
    let first_only = compile(r"[a-z]+", "").unwrap();
    let out = first_only
        .replace_with("hi there", |m| Ok::<String, ()>(m.value.to_uppercase()))
        .unwrap();
    assert_eq!(out, "HI there");
}

#[test]
fn replace_with_stops_at_the_first_callback_error() {
    let re = compile(r"\d", "g").unwrap();
    let err = re
        .replace_with("a1 b2", |_| Err::<String, &str>("boom"))
        .expect_err("the callback failed");
    assert_eq!(err, "boom");
}

#[test]
fn multiline_flag_makes_anchors_match_line_edges() {
    let anchored = compile(r"^\d+$", "m").unwrap();
    assert!(anchored.test("abc\n42\ndef"));
    let single = compile(r"^\d+$", "").unwrap();
    assert!(!single.test("abc\n42\ndef"));
}
