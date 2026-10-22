//! The `io` namespace: the host's standard streams — stdout, stderr, and
//! stdin — reached as `std::io::…`. Like [`crate::fs`], every member is
//! fallible and reports its failure as a message string (`Result<T, Error>`
//! in the language, wrapped by the interpreter with the built-in `Error`).
//!
//! These operations are the real streams, unlike the language's `print`,
//! which the interpreter collects into a buffer the host prints once the
//! program ends: stdout is written and flushed as the program runs — so a
//! prompt is visible before `read_line` blocks — and stdin is read line by
//! line. The `_to` / `_from` cores take their stream from the caller, which
//! is how the integration tests capture bytes without touching the process
//! streams.

use std::io::{self, BufRead, Write};

/// The failure message of an operation: which stream it was on, and the
/// underlying OS error (`cannot write to stdout: broken pipe (os error 32)`).
fn fail(op: &str, err: io::Error) -> String {
    format!("cannot {op}: {err}")
}

/// The write core behind `print` / `println` / `eprint` / `eprintln`:
/// `stream` names the destination for the failure message. Every write
/// flushes before it returns, so output cannot sit in a buffer while a
/// later `read_line` waits for input.
pub fn write_to(out: &mut impl Write, stream: &str, text: &str) -> Result<(), String> {
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|e| fail(&format!("write to {stream}"), e))
}

/// [`write_to`] plus the line terminator: `text` and a `\n`.
pub fn write_line_to(out: &mut impl Write, stream: &str, text: &str) -> Result<(), String> {
    out.write_all(text.as_bytes())
        .and_then(|()| out.write_all(b"\n"))
        .and_then(|()| out.flush())
        .map_err(|e| fail(&format!("write to {stream}"), e))
}

/// The flush core behind `std::io::flush()`.
pub fn flush_to(out: &mut impl Write, stream: &str) -> Result<(), String> {
    out.flush().map_err(|e| fail(&format!("flush {stream}"), e))
}

/// The read core behind `std::io::readLine()`: the next line without its
/// line ending (`\n`, or `\r\n`), or `None` at end of input — the `null` a
/// program loops on. An empty line is `Some("")`, so it never passes for EOF.
pub fn read_line_from(input: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut buf = Vec::new();
    let read = input
        .read_until(b'\n', &mut buf)
        .map_err(|e| fail("read from stdin", e))?;
    if read == 0 {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    }
    String::from_utf8(buf).map(Some).map_err(|e| {
        format!(
            "cannot read from stdin: invalid UTF-8 at byte {}",
            e.utf8_error().valid_up_to()
        )
    })
}

/// The read core behind `std::io::read()`: the whole stream as a string.
/// Reports UTF-8 exactly like [`read_line_from`] does, so the two reads agree
/// on how a non-text stream is described.
pub fn read_from(input: &mut impl io::Read) -> Result<String, String> {
    let mut bytes = Vec::new();
    input
        .read_to_end(&mut bytes)
        .map_err(|e| fail("read from stdin", e))?;
    String::from_utf8(bytes).map_err(|e| {
        format!(
            "cannot read from stdin: invalid UTF-8 at byte {}",
            e.utf8_error().valid_up_to()
        )
    })
}

/// `std::io::print(text)`: `text` on stdout with no line terminator, flushed
/// before the call returns. Takes a `string` — `str(value)` renders anything
/// else, exactly as the language's `print` does.
pub fn print(text: &str) -> Result<(), String> {
    write_to(&mut io::stdout().lock(), "stdout", text)
}

/// `std::io::println(text)`: `text` and a newline on stdout, flushed.
pub fn println(text: &str) -> Result<(), String> {
    write_line_to(&mut io::stdout().lock(), "stdout", text)
}

/// `std::io::eprint(text)`: `text` on stderr with no line terminator.
pub fn eprint(text: &str) -> Result<(), String> {
    write_to(&mut io::stderr().lock(), "stderr", text)
}

/// `std::io::eprintln(text)`: `text` and a newline on stderr.
pub fn eprintln(text: &str) -> Result<(), String> {
    write_line_to(&mut io::stderr().lock(), "stderr", text)
}

/// `std::io::flush()`: push anything still buffered on stdout to the
/// operating system. Writes here flush themselves; this is for output a
/// program reached through other means.
pub fn flush() -> Result<(), String> {
    flush_to(&mut io::stdout().lock(), "stdout")
}

/// `std::io::readLine()`: the next line of stdin without its terminator, or
/// `Ok(null)` at end of input — an empty line reads as `Ok("")`. Bytes that
/// are not valid UTF-8 are a failure.
pub fn read_line() -> Result<Option<String>, String> {
    read_line_from(&mut io::stdin().lock())
}

/// `std::io::read()`: all of stdin as a string (`""` at end of input); bytes
/// that are not valid UTF-8 are a failure.
pub fn read() -> Result<String, String> {
    read_from(&mut io::stdin().lock())
}
