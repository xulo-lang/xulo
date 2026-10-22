//! The `io` namespace semantics in isolation — no language involved, just the
//! stream primitives and their failure messages. Every test drives a memory
//! stream (or a deliberately broken one), so the process's own stdout/stderr/
//! stdin stay untouched.

use std::io::{self as stdio, BufRead, Cursor, Read, Write};

use xulo_std::io;

/// A writer that refuses everything: the failure path every write shares.
struct Broken;

impl Write for Broken {
    fn write(&mut self, _buf: &[u8]) -> stdio::Result<usize> {
        Err(stdio::Error::other("broken"))
    }

    fn flush(&mut self) -> stdio::Result<()> {
        Err(stdio::Error::other("broken"))
    }
}

/// A reader that fails the way a disconnected stdin does.
struct BrokenRead;

impl Read for BrokenRead {
    fn read(&mut self, _buf: &mut [u8]) -> stdio::Result<usize> {
        Err(stdio::Error::other("broken"))
    }
}

impl BufRead for BrokenRead {
    fn fill_buf(&mut self) -> stdio::Result<&[u8]> {
        Err(stdio::Error::other("broken"))
    }

    fn consume(&mut self, _amt: usize) {}
}

#[test]
fn print_writes_the_text_untouched() {
    let mut out = Vec::new();
    io::write_to(&mut out, "stdout", "abc").unwrap();
    assert_eq!(out, b"abc");
}

#[test]
fn println_appends_a_single_newline() {
    let mut out = Vec::new();
    io::write_line_to(&mut out, "stdout", "abc").unwrap();
    assert_eq!(out, b"abc\n");

    let mut empty = Vec::new();
    io::write_line_to(&mut empty, "stdout", "").unwrap();
    assert_eq!(empty, b"\n");
}

#[test]
fn a_write_names_the_stream_and_the_os_error() {
    let err = io::write_to(&mut Broken, "stdout", "x").expect_err("broken writer");
    assert_eq!(err, "cannot write to stdout: broken");

    let err = io::write_line_to(&mut Broken, "stderr", "x").expect_err("broken writer");
    assert_eq!(err, "cannot write to stderr: broken");
}

#[test]
fn flush_reports_the_stream_it_failed_on() {
    let mut out = Vec::new();
    io::flush_to(&mut out, "stdout").unwrap();

    let err = io::flush_to(&mut Broken, "stdout").expect_err("broken writer");
    assert_eq!(err, "cannot flush stdout: broken");
}

#[test]
fn read_line_drops_both_line_endings() {
    let mut input = Cursor::new(b"one\ntwo\r\n");
    assert_eq!(io::read_line_from(&mut input).unwrap(), Some("one".into()));
    assert_eq!(io::read_line_from(&mut input).unwrap(), Some("two".into()));
    assert_eq!(io::read_line_from(&mut input).unwrap(), None);
}

#[test]
fn an_empty_line_is_not_end_of_input() {
    let mut input = Cursor::new(b"\n\n");
    assert_eq!(io::read_line_from(&mut input).unwrap(), Some(String::new()));
    assert_eq!(io::read_line_from(&mut input).unwrap(), Some(String::new()));
    assert_eq!(io::read_line_from(&mut input).unwrap(), None);
}

#[test]
fn a_final_line_without_a_newline_still_reads() {
    let mut input = Cursor::new(b"tail");
    assert_eq!(io::read_line_from(&mut input).unwrap(), Some("tail".into()));
    assert_eq!(io::read_line_from(&mut input).unwrap(), None);
}

#[test]
fn read_line_names_stdin_and_the_os_error() {
    let err = io::read_line_from(&mut BrokenRead).expect_err("broken reader");
    assert_eq!(err, "cannot read from stdin: broken");
}

#[test]
fn read_takes_every_remaining_byte() {
    let mut input = Cursor::new(b"all of it");
    assert_eq!(io::read_from(&mut input).unwrap(), "all of it");

    let mut empty = Cursor::new(Vec::<u8>::new());
    assert_eq!(io::read_from(&mut empty).unwrap(), "");
}

#[test]
fn read_reports_invalid_utf8_as_a_failure() {
    let mut input = Cursor::new(vec![0xff, 0xfe]);
    let err = io::read_from(&mut input).expect_err("not utf-8");
    assert!(
        err.starts_with("cannot read from stdin: invalid UTF-8"),
        "unexpected: {err}"
    );
}

#[test]
fn read_names_stdin_and_the_os_error() {
    let err = io::read_from(&mut BrokenRead).expect_err("broken reader");
    assert_eq!(err, "cannot read from stdin: broken");
}
