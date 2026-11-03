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

use std::cell::RefCell;
use std::io::{self, BufRead, IsTerminal, Read, Write};

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

/// The read core behind `std::io::read_line()`: the next line without its
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

/// `std::io::read_line()`: the next line of stdin without its terminator, or
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

/// The maximum byte count a single `read_n` accepts (spec `modules/io.md`).
pub const MAX_READ_N: i64 = 16 * 1024 * 1024;

thread_local! {
    /// Bytes read from stdin but held back because they were an incomplete
    /// UTF-8 sequence at the previous cut (spec `modules/io.md`).
    static PENDING: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// `std::io::read_n(max)`: up to `max` bytes of stdin decoded as UTF-8, or
/// `Ok(null)` at end of input. The read stops at a character boundary; an
/// incomplete trailing sequence is kept for the next call. A `max` outside
/// `1 …= 16777216` is an error (spec `modules/io.md`).
pub fn read_n(max: i64) -> Result<Option<String>, String> {
    if !(1..=MAX_READ_N).contains(&max) {
        return Err(format!(
            "std::io::read_n: max must be in 1..=16777216, found {max}"
        ));
    }
    let max = max as usize;
    let mut out = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
    let mut stdin = io::stdin().lock();
    while out.len() < max {
        let mut byte = [0u8; 1];
        match stdin.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => out.push(byte[0]),
            Err(e) => return Err(format!("cannot read from stdin: {e}")),
        }
    }
    if out.is_empty() {
        return Ok(None);
    }
    match std::str::from_utf8(&out) {
        Ok(text) => Ok(Some(text.to_string())),
        Err(e) if e.error_len().is_none() => {
            let valid = e.valid_up_to();
            if valid == 0 {
                // A single character longer than `max`: hold it for a larger
                // read rather than splitting it.
                PENDING.with(|p| *p.borrow_mut() = out);
                return Err(
                    "cannot read from stdin: incomplete UTF-8 sequence at the read limit"
                        .to_string(),
                );
            }
            let text = String::from_utf8(out[..valid].to_vec()).map_err(|e| {
                format!(
                    "cannot read from stdin: invalid UTF-8 at byte {}",
                    e.utf8_error().valid_up_to()
                )
            })?;
            PENDING.with(|p| *p.borrow_mut() = out[valid..].to_vec());
            Ok(Some(text))
        }
        Err(e) => Err(format!(
            "cannot read from stdin: invalid UTF-8 at byte {}",
            e.valid_up_to()
        )),
    }
}

/// `std::io::stdin_is_tty()`: whether stdin is a terminal. Total.
pub fn stdin_is_tty() -> bool {
    io::stdin().is_terminal()
}

/// `std::io::stdout_is_tty()`: whether stdout is a terminal. Total.
pub fn stdout_is_tty() -> bool {
    io::stdout().is_terminal()
}
