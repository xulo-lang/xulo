//! `std::fmt::format` in isolation — the directive engine's own semantics
//! with no Xulo pipeline involved: every checked example of
//! `std-spec/modules/fmt.md`, the flag/width/precision rules, the operand
//! domains, and the runtime-failure shapes. The interpreter's `Value` and the
//! compiled tagged-payload conversions live in `xulo-runtime`.

use xulo_std::fmt::{FmtArg, FmtText, format};

fn s(text: &str) -> FmtArg<'static> {
    FmtArg::Str(text.to_string())
}

fn i(n: i64) -> FmtArg<'static> {
    FmtArg::Int(n)
}

fn f(x: f64) -> FmtArg<'static> {
    FmtArg::Float(x)
}

/// A user value whose implementation answers (no precision cut follows).
fn consulted(text: &'static str) -> FmtArg<'static> {
    FmtArg::User {
        kind: "a struct",
        render: Box::new(move |precision, alternate| {
            Ok(FmtText::Consulted(format!(
                "{text}:{precision:?}:{alternate}"
            )))
        }),
    }
}

/// A value with no implementation: the table text is cut by precision.
fn table(text: &'static str) -> FmtArg<'static> {
    FmtArg::User {
        kind: "a struct",
        render: Box::new(move |_, _| Ok(FmtText::Table(text.to_string()))),
    }
}

fn ok(template: &str, args: Vec<FmtArg<'_>>) -> String {
    format(template, &args).expect("well-formed")
}

fn bad(template: &str, args: Vec<FmtArg<'_>>) -> String {
    format(template, &args).expect_err("runtime failure")
}

#[test]
fn literal_text_and_escapes_come_through() {
    assert_eq!(ok("100%%", vec![]), "100%");
    assert_eq!(ok("hi", vec![]), "hi");
    assert_eq!(ok("a %% b %%", vec![]), "a % b %");
    assert_eq!(ok("", vec![]), "");
}

#[test]
fn text_verbs_render_the_str_table() {
    assert_eq!(ok("%s", vec![s("hi")]), "hi");
    assert_eq!(ok("%v", vec![s("hi")]), "hi");
    assert_eq!(ok("%v", vec![i(42)]), "42");
    assert_eq!(ok("%v", vec![f(3.0)]), "3");
    assert_eq!(ok("%v", vec![f(2.5)]), "2.5");
    assert_eq!(ok("%v", vec![FmtArg::Bool(true)]), "true");
    assert_eq!(ok("%v", vec![FmtArg::Null]), "null");
    assert_eq!(ok("%s", vec![FmtArg::Rendered("[1, 2]".into())]), "[1, 2]");
    assert_eq!(
        ok("%t / %v", vec![FmtArg::Bool(true), FmtArg::Null]),
        "true / null"
    );
    // A written precision cuts a table rendering for both text verbs.
    assert_eq!(
        ok("%.3v", vec![FmtArg::Rendered("[1, 2, 3]".into())]),
        "[1,"
    );
    assert_eq!(
        ok("%.3s", vec![FmtArg::Rendered("[1, 2, 3]".into())]),
        "[1,"
    );
}

#[test]
fn text_verbs_consult_user_values_under_the_directive_format() {
    // An implementation's return is used as returned: precision is handed
    // over, never re-applied as a cut.
    assert_eq!(
        ok("%.1s", vec![consulted("123.45")]),
        "123.45:Some(1):false"
    );
    assert_eq!(ok("%#v", vec![consulted("x")]), "x:None:true");
    assert_eq!(ok("%.2s", vec![table("abcdef")]), "ab");
}

#[test]
fn q_quotes_a_xulo_string_literal() {
    assert_eq!(ok("%q", vec![s("hi")]), r#""hi""#);
    assert_eq!(ok("%q", vec![s("a\n")]), r#""a\n""#);
    assert_eq!(ok("%q", vec![s("a\tb")]), r#""a\tb""#);
    assert_eq!(ok("%q", vec![s("a\\b")]), r#""a\\b""#);
    assert_eq!(ok("%q", vec![s("\u{1}")]), r#""\u0001""#);
    assert_eq!(ok("%q", vec![s("\u{7f}")]), r#""\u007f""#);
    assert_eq!(ok("%q", vec![s("€")]), r#""€""#);
}

#[test]
fn c_takes_one_scalar_and_checks_its_range() {
    assert_eq!(ok("%c", vec![i(8364)]), "€");
    assert_eq!(ok("%c", vec![i(65)]), "A");
    assert_eq!(ok("%c", vec![i(0)]), "\u{0}");
    assert_eq!(
        bad("%c", vec![i(0x110000)]),
        "`std::fmt::format` `%c` scalar out of range: 1114112 is not in 0 …= 0x10FFFF"
    );
    assert_eq!(
        bad("%c", vec![i(0xD800)]),
        "`std::fmt::format` `%c` scalar out of range: 55296 is a surrogate"
    );
    let err = bad("%c", vec![s("a")]);
    assert!(
        err.contains("`%c` takes an int value, found a string"),
        "unexpected: {err}"
    );
}

#[test]
fn t_takes_a_bool() {
    assert_eq!(ok("%t", vec![FmtArg::Bool(false)]), "false");
    let err = bad("%t", vec![i(1)]);
    assert!(
        err.contains("`%t` takes a bool value, found an int"),
        "unexpected: {err}"
    );
}

#[test]
fn integer_verbs_take_the_absolute_value_and_keep_the_sign() {
    assert_eq!(ok("%d", vec![i(-42)]), "-42");
    assert_eq!(ok("%d", vec![i(42)]), "42");
    assert_eq!(ok("%x / %#X", vec![i(255), i(255)]), "ff / 0XFF");
    assert_eq!(ok("%b", vec![i(5)]), "101");
    assert_eq!(ok("%o", vec![i(64)]), "100");
    assert_eq!(ok("%#x", vec![i(255)]), "0xff");
    assert_eq!(ok("%#o", vec![i(64)]), "0o100");
    assert_eq!(ok("%#b", vec![i(5)]), "0b101");
    assert_eq!(ok("%#X", vec![i(255)]), "0XFF");
    // The one value with no positive counterpart digitizes from its
    // two's-complement image.
    assert_eq!(ok("%x", vec![i(i64::MIN)]), "-8000000000000000");
    assert_eq!(ok("%d", vec![i(i64::MIN)]), "-9223372036854775808");
}

#[test]
fn integer_verbs_accept_a_whole_float_value() {
    // The runtime has one number model: `2` and `2.0` are the same value.
    assert_eq!(ok("%d", vec![f(2.0)]), "2");
    assert_eq!(ok("%x", vec![f(-255.0)]), "-ff");
}

#[test]
fn integer_verbs_refuse_a_fractional_number() {
    let err = bad("%d", vec![f(2.5)]);
    assert!(
        err.contains("`%d` takes an int value, found a float"),
        "unexpected: {err}"
    );
    let err = bad("%d", vec![s("a")]);
    assert!(
        err.contains("`%d` takes an int value, found a string"),
        "unexpected: {err}"
    );
}

#[test]
#[allow(clippy::approx_constant)]
fn fixed_point_defaults_to_six_and_rounds_half_away() {
    assert_eq!(ok("%f", vec![f(2.5)]), "2.500000");
    assert_eq!(ok("%.2f", vec![f(3.14159)]), "3.14");
    assert_eq!(ok("%.0f", vec![f(2.5)]), "3");
    assert_eq!(ok("%.0f", vec![f(-2.5)]), "-3");
    assert_eq!(ok("%f", vec![i(42)]), "42.000000");
}

#[test]
fn scientific_one_digit_point_precision_then_exponent() {
    assert_eq!(ok("%e", vec![f(1234.5)]), "1.234500e+03");
    assert_eq!(ok("%E", vec![f(1234.5)]), "1.234500E+03");
    assert_eq!(ok("%.0e", vec![f(1234.5)]), "1e+03");
    assert_eq!(ok("%e", vec![f(0.0)]), "0.000000e+00");
    assert_eq!(ok("%.2e", vec![f(-0.001)]), "-1.00e-03");
    assert_eq!(ok("%e", vec![f(0.0001)]), "1.000000e-04");
}

#[test]
fn g_takes_significant_digits_and_picks_the_shorter_form() {
    assert_eq!(ok("%.3g", vec![f(0.1)]), "0.1");
    assert_eq!(ok("%g", vec![f(1e-7)]), "1e-07");
    assert_eq!(ok("%g", vec![f(100000.0)]), "100000");
    assert_eq!(ok("%g", vec![f(1e6)]), "1e+06");
    assert_eq!(ok("%g", vec![f(123.45)]), "123.45");
    assert_eq!(ok("%G", vec![f(1e-7)]), "1E-07");
    assert_eq!(ok("%.5g", vec![f(123000.0)]), "1.23e+05");
    assert_eq!(ok("%.3g", vec![f(999.9999)]), "1e+03");
    assert_eq!(ok("%.3g", vec![f(120.4)]), "120");
    assert_eq!(ok("%.1g", vec![f(0.25)]), "0.3");
    assert_eq!(ok("%.4g", vec![f(0.0001)]), "0.0001");
    assert_eq!(ok("%.5g", vec![f(0.00001)]), "1e-05");
    assert_eq!(ok("%g", vec![f(0.0)]), "0");
}

#[test]
fn special_values_ignore_precision_and_numeric_flags() {
    assert_eq!(ok("%f", vec![f(f64::NAN)]), "NaN");
    assert_eq!(ok("%f", vec![f(f64::INFINITY)]), "Infinity");
    assert_eq!(ok("%f", vec![f(f64::NEG_INFINITY)]), "-Infinity");
    assert_eq!(ok("%.3f", vec![f(f64::NAN)]), "NaN");
    assert_eq!(ok("%+.3f", vec![f(f64::INFINITY)]), "Infinity");
    assert_eq!(ok("%08.2f", vec![f(f64::NAN)]), "     NaN");
    assert_eq!(ok("%-8s", vec![FmtArg::Float(f64::NAN)]), "NaN     ");
    assert_eq!(ok("%v", vec![f(f64::NAN)]), "NaN");
    assert_eq!(ok("%v", vec![f(f64::NEG_INFINITY)]), "-Infinity");
}

#[test]
fn negative_zero_keeps_its_sign() {
    assert_eq!(ok("%f", vec![f(-0.0)]), "-0.000000");
    assert_eq!(ok("%e", vec![f(-0.0)]), "-0.000000e+00");
    assert_eq!(ok("%g", vec![f(-0.0)]), "-0");
}

#[test]
#[allow(clippy::approx_constant)]
fn width_is_a_scalar_minimum_with_left_and_zero_padding() {
    assert_eq!(ok("%-8s|", vec![s("hi")]), "hi      |");
    assert_eq!(ok("%-6s|", vec![s("hi")]), "hi    |");
    assert_eq!(ok("%5s", vec![s("hi")]), "   hi");
    assert_eq!(ok("%5s", vec![s("hello")]), "hello");
    assert_eq!(ok("%6.2f", vec![f(3.14159)]), "  3.14");
    assert_eq!(ok("%+8.2f", vec![f(3.1)]), "   +3.10");
    assert_eq!(ok("%08d", vec![i(42)]), "00000042");
    assert_eq!(ok("%08d", vec![i(-42)]), "-0000042");
    assert_eq!(ok("%08.2f", vec![f(-3.1)]), "-0003.10");
    assert_eq!(ok("%#08x", vec![i(255)]), "0x0000ff");
    assert_eq!(ok("%5c", vec![i(8364)]), "    €");
    assert_eq!(ok("%-5c|", vec![i(8364)]), "€    |");
}

#[test]
fn plus_outranks_space_and_dash_ignores_zero() {
    assert_eq!(ok("%+d", vec![i(42)]), "+42");
    assert_eq!(ok("% d", vec![i(42)]), " 42");
    assert_eq!(ok("%+ d", vec![i(42)]), "+42");
    assert_eq!(ok("%-05d", vec![i(42)]), "42   ");
    assert_eq!(ok("%+d", vec![i(-42)]), "-42");
}

#[test]
fn s_and_q_take_a_precision_cut() {
    assert_eq!(ok("%.3s", vec![s("hello")]), "hel");
    assert_eq!(ok("%.10s", vec![s("hello")]), "hello");
    assert_eq!(ok("%.1q", vec![s("hi")]), r#""h""#);
    assert_eq!(ok("%.3s", vec![s("€€x")]), "€€x");
}

#[test]
fn template_directives_and_arguments_must_agree() {
    let err = bad("%d", vec![]);
    assert!(
        err.contains("expects 1 argument, found 0"),
        "unexpected: {err}"
    );
    let err = bad("a %d b %d", vec![i(1)]);
    assert!(
        err.contains("expects 2 arguments, found 1"),
        "unexpected: {err}"
    );
    let err = bad("100%%", vec![i(1)]);
    assert!(
        err.contains("expects 0 arguments, found 1"),
        "unexpected: {err}"
    );
    let err = bad("%d", vec![i(1), i(2)]);
    assert!(
        err.contains("expects 1 argument, found 2"),
        "unexpected: {err}"
    );
}

#[test]
fn malformed_templates_are_runtime_failures() {
    assert!(bad("%z", vec![i(1)]).contains("unknown verb `z`"));
    assert!(bad("%", vec![]).contains("`%` with no verb"));
    assert!(bad("a %", vec![]).contains("`%` with no verb"));
    assert!(bad("%--d", vec![i(1)]).contains("repeated flag `-`"));
    assert!(bad("%00d", vec![i(1)]).contains("repeated flag `0`"));
    assert!(bad("%+5%%", vec![]).contains("`%%` takes no flags"));
    assert!(bad("%5.", vec![]).contains("`.` must be followed"));
    assert!(bad("%.d", vec![]).contains("`.` must be followed"));
    assert!(bad("%#d", vec![i(1)]).contains("flag `#` is not valid for verb `d`"));
    assert_eq!(ok("%#v", vec![i(1)]), "1");
    assert_eq!(ok("%#s", vec![s("x")]), "x");
    assert!(bad("%+s", vec![s("x")]).contains("flag `+` is not valid for verb `s`"));
    assert!(bad("% s", vec![s("x")]).contains("flag ` ` is not valid for verb `s`"));
    assert!(bad("%0s", vec![s("x")]).contains("flag `0` is not valid for verb `s`"));
    assert!(bad("%.3t", vec![FmtArg::Bool(true)]).contains("precision is not valid for verb `t`"));
    assert!(bad("%.16f", vec![f(1.0)]).contains("precision out of range for verb `f`: 0 to 15"));
    assert!(bad("%.0g", vec![f(1.0)]).contains("precision out of range for verb `g`: 1 to 17"));
    assert!(bad("%.18g", vec![f(1.0)]).contains("precision out of range for verb `g`: 1 to 17"));
}

#[test]
fn q_requires_a_string_operand() {
    let err = bad("%q", vec![i(1)]);
    assert!(
        err.contains("`%q` takes a string, found an int"),
        "unexpected: {err}"
    );
    let err = bad("%q", vec![FmtArg::Rendered("[1]".into())]);
    assert!(
        err.contains("`%q` takes a string, found a rendered value"),
        "unexpected: {err}"
    );
}

#[test]
#[allow(clippy::approx_constant)]
fn checked_call_examples_of_the_spec() {
    assert_eq!(ok("%6.2f", vec![f(3.14159)]), "  3.14");
    assert_eq!(ok("100%%", vec![]), "100%");
    assert_eq!(ok("%-8s|%d", vec![s("hi"), i(42)]), "hi      |42");
    assert_eq!(ok("%#08x", vec![i(255)]), "0x0000ff");
    assert_eq!(ok("%.3g, %q", vec![f(0.1), s("hi")]), r#"0.1, "hi""#);
}
