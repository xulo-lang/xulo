use xulo_core::diagnostics;
use xulo_core::error::{XuloError, ErrorKind};

#[test]
fn test_locate_first_line() {
    let src = "hello world";
    let (line, col, text) = diagnostics::locate(src, 0);
    assert_eq!(line, 1);
    assert_eq!(col, 1);
    assert_eq!(text, "hello world");
}

#[test]
fn test_locate_middle_of_line() {
    let src = "hello world";
    let (line, col, text) = diagnostics::locate(src, 5);
    assert_eq!(line, 1);
    assert_eq!(col, 6);
    assert_eq!(text, "hello world");
}

#[test]
fn test_locate_second_line() {
    let src = "line1\nline2\nline3";
    let (line, col, text) = diagnostics::locate(src, 6);
    assert_eq!(line, 2);
    assert_eq!(col, 1);
    assert_eq!(text, "line2");
}

#[test]
fn test_locate_third_line_middle() {
    let src = "line1\nline2\nline3";
    let (line, col, text) = diagnostics::locate(src, 14);
    assert_eq!(line, 3);
    assert_eq!(col, 3);
    assert_eq!(text, "line3");
}

#[test]
fn test_locate_end_of_file() {
    let src = "abc";
    let (line, col, text) = diagnostics::locate(src, 3);
    assert_eq!(line, 1);
    assert_eq!(col, 4);
    assert_eq!(text, "abc");
}

#[test]
fn test_render_error_no_span() {
    diagnostics::use_color(false);
    let err = XuloError::new(ErrorKind::Parse, "unexpected token");
    let rendered = diagnostics::render(&err, None);
    assert!(rendered.contains("error"));
    assert!(rendered.contains("syntax"));
    assert!(rendered.contains("unexpected token"));
}

#[test]
fn test_render_warning_no_span() {
    diagnostics::use_color(false);
    let err = XuloError::new(ErrorKind::Warning, "unused variable");
    let rendered = diagnostics::render(&err, None);
    assert!(rendered.contains("warning"));
    assert!(rendered.contains("unused variable"));
}

#[test]
fn test_render_error_with_span_and_source() {
    diagnostics::use_color(false);
    let err = XuloError::new(ErrorKind::Semantic, "undefined variable 'x'")
        .at(15..16);
    let source = "fn main() {\n  print(x)\n}";
    let rendered = diagnostics::render(&err, Some(source));
    assert!(rendered.contains("error"));
    assert!(rendered.contains("semantic"));
    assert!(rendered.contains("undefined variable 'x'"));
    assert!(rendered.contains("-->"));
}

#[test]
fn test_render_error_with_file() {
    diagnostics::use_color(false);
    let err = XuloError::new(ErrorKind::Io, "file not found")
        .with_file(std::path::PathBuf::from("test.xulo"));
    let rendered = diagnostics::render(&err, None);
    assert!(rendered.contains("test.xulo"));
}

#[test]
fn test_use_color_disabled() {
    diagnostics::use_color(false);
    let err = XuloError::new(ErrorKind::Parse, "test");
    let rendered = diagnostics::render(&err, None);
    assert!(!rendered.contains("\x1b["));
}

#[test]
fn test_locate_multiline_complex() {
    let src = "fn add(a, b) {\n  let result = a + b\n  return result\n}";
    let (line, col, text) = diagnostics::locate(src, 25);
    assert_eq!(line, 2);
    assert!(col > 1);
    assert!(text.starts_with("  let"));
}
