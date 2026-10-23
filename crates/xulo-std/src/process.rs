//! The `process` namespace: process-level control (`exit`, `args`) and
//! synchronous child-process capture (`run`) — implementation-first (spec
//! `builtins/intrinsic-functions.md` is Math/Time-only).
//!
//! Like `std::net` v1, `run` is **synchronous and blocking**: it waits for
//! the child to finish before returning, freezing the cooperative scheduler
//! for the duration (there is no evented child I/O in the runtime yet).
//! Commands run through the POSIX shell (`sh -c`); a spawn the shell
//! refuses (no `sh` on the host) is a clean `Err`, never a crash.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

/// Per-stream capture bound for `run`: a command that produces more than
/// this fails with an `Err` instead of growing the host process without
/// limit (same order of magnitude as the string/read caps elsewhere).
const MAX_CAPTURE_BYTES: usize = 1 << 24;

/// Terminate the current process with `code` (OS-truncated on Unix, so
/// `256+` wraps like any C `exit`). Output is flushed first — buffered
/// stdout/stderr lines must reach the terminal before the process dies.
/// Never returns; the interpreter drains its buffered `print` lines before
/// calling this (see `native_process_exit`).
pub fn exit(code: i32) -> ! {
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    std::process::exit(code)
}

/// `std::process::args()`: the process's argument vector (`argv`), starting
/// with the program path — the script or binary that started this process.
pub fn args() -> Vec<String> {
    std::env::args().collect()
}

/// One `std::process::run` capture: both streams decoded lossily (child
/// output is bytes; invalid UTF-8 surfaces as replacement characters, same
/// as `sock_read`) plus the exit status (`-1` when a signal killed the
/// child, which has no code).
#[derive(Debug)]
pub struct RunOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: i64,
}

/// Read one piped stream up to `MAX_CAPTURE_BYTES + 1` bytes so the caller
/// can tell "exactly at the cap" from "past the cap". Runs on either this
/// thread or the stderr helper thread.
fn read_capped<R: Read>(mut src: R) -> (Vec<u8>, bool) {
    let mut buf = Vec::new();
    let mut limited = (&mut src).take(MAX_CAPTURE_BYTES as u64 + 1);
    let _ = limited.read_to_end(&mut buf);
    let over = buf.len() > MAX_CAPTURE_BYTES;
    if over {
        buf.truncate(MAX_CAPTURE_BYTES);
    }
    (buf, over)
}

/// `std::process::run(cmd[, input])`: run `cmd` through `sh -c` to
/// completion, capturing both streams — `Ok` of stdout/stderr/exit code
/// even when the command fails (a non-zero exit is data, not an error);
/// `Err` only when the shell itself cannot be started or a stream crosses
/// the capture cap. With `input`, the child's stdin is the string (closed
/// after the write); without it, the child inherits this process's stdin.
///
/// Both streams drain concurrently and a reader that crosses the cap kills
/// the child first — otherwise a full pipe would leave it blocked on write
/// and `wait` would deadlock. The child handle is shared behind a mutex
/// purely so either reader thread can kill it.
pub fn run(cmd: &str, input: Option<&str>) -> Result<RunOutput, String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::inherit()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run `{cmd}`: {e}"))?;

    let out_pipe = child.stdout.take().expect("stdout is piped");
    let err_pipe = child.stderr.take().expect("stderr is piped");
    let stdin_pipe = child.stdin.take();
    let child = Arc::new(Mutex::new(child));

    // stderr drains on its own thread so a chatty child cannot fill that
    // pipe against this thread's stdout read; over the cap it kills the
    // child (which EOFs stdout) instead of letting both sides wedge.
    let killer = Arc::clone(&child);
    let err_handle = std::thread::spawn(move || {
        let (buf, over) = read_capped(err_pipe);
        if over {
            let _ = killer.lock().unwrap().kill();
        }
        (buf, over)
    });

    if let Some(data) = input {
        let Some(mut stdin) = stdin_pipe else {
            return Err(format!("stdin of `{cmd}` is not piped"));
        };
        let write = stdin.write_all(data.as_bytes());
        drop(stdin); // EOF: `cat`-style children stop at the end of input
        if let Err(e) = write {
            let _ = child.lock().unwrap().kill();
            let _ = err_handle.join();
            let _ = child.lock().unwrap().wait();
            return Err(format!("cannot write stdin of `{cmd}`: {e}"));
        }
    }

    let (out_bytes, out_over) = read_capped(out_pipe);
    if out_over {
        let _ = child.lock().unwrap().kill();
    }
    let (err_bytes, err_over) = err_handle
        .join()
        .map_err(|_| format!("stderr reader of `{cmd}` panicked"))?;

    // Reap (kill paths above have already signalled; this collects the code).
    let status = child
        .lock()
        .unwrap()
        .wait()
        .map_err(|e| format!("cannot wait for `{cmd}`: {e}"))?;
    if out_over || err_over {
        return Err(format!(
            "`{cmd}` output exceeds {MAX_CAPTURE_BYTES} bytes per stream"
        ));
    }

    Ok(RunOutput {
        stdout: String::from_utf8_lossy(&out_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&err_bytes).into_owned(),
        code: status.code().map(i64::from).unwrap_or(-1),
    })
}
