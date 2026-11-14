//! The `process` namespace (spec `std-spec/modules/process.md`): process-level
//! control (`exit`, `args`) and the whole child-process surface — the one-shot
//! captures `run`/`exec`, the kept handle `launch`, and the `Command`
//! specification both shorthands are built from. Like `std::net`, these
//! members block: a `timeout` budget is how a program bounds that freeze.
//!
//! Failures stay data: a non-zero child exit is `Ok` with `code != 0`, and
//! only four classes reach `Err` — the child cannot start, a captured stream
//! crosses the 16 MiB cap, the budget expires, or a `Child` handle cannot
//! serve the call (spent, or already exited when asked to kill).

use std::io::{Read, Write};
use std::process::{
    Child as StdChild, ChildStderr, ChildStdin, ChildStdout, Command as StdCommand, ExitStatus,
    Stdio,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Per-stream capture bound for `run`/`exec`/`Child::wait`: a command that
/// produces more than this fails with an `Err` instead of growing the host
/// process without limit (same order of magnitude as the string/read caps
/// elsewhere). The cap is per stream — exactly at the bound is captured whole.
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

/// One capture record (`run`, `exec`, `Command::run`, `Child::wait`): both
/// streams decoded lossily (child output is bytes; invalid UTF-8 surfaces as
/// replacement characters, same as `sock_read`) plus the exit status (`-1`
/// when a signal killed the child, which has no code).
#[derive(Debug)]
pub struct RunOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: i64,
}

/// Read one piped stream up to `MAX_CAPTURE_BYTES + 1` bytes so the caller
/// can tell "exactly at the cap" from "past the cap". Runs on either stream's
/// reader thread.
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

/// How a child starts: through the platform shell (the `run`/`launch` form,
/// so shell syntax is part of the command's meaning), or a program with its
/// own argument vector (the `exec` form — no shell, nothing to quote).
#[derive(Clone, Debug)]
pub enum CommandKind {
    Shell(String),
    Direct { program: String, args: Vec<String> },
}

/// One child's full specification: how to start it plus the optional stdin
/// string, working directory, environment overlay, and lifetime budget. The
/// `Command` setters are pure — each `with_*`/`env` takes the spec by value
/// and returns the refined clone, so the receiver never changes.
#[derive(Clone, Debug)]
pub struct CommandSpec {
    pub kind: CommandKind,
    pub input: Option<String>,
    pub cwd: Option<String>,
    pub envs: Vec<(String, String)>,
    pub timeout: Option<u64>,
}

#[cfg(windows)]
fn shell_command(line: &str) -> StdCommand {
    let mut c = StdCommand::new("cmd");
    c.arg("/C").arg(line);
    c
}

#[cfg(not(windows))]
fn shell_command(line: &str) -> StdCommand {
    let mut c = StdCommand::new("sh");
    c.arg("-c").arg(line);
    c
}

impl CommandSpec {
    /// `Command::shell(cmd)`: the spec behind `run(cmd)` — `cmd` runs through
    /// `sh -c` on Unix and `cmd /C` on Windows.
    pub fn shell(cmd: &str) -> Self {
        Self {
            kind: CommandKind::Shell(cmd.to_string()),
            input: None,
            cwd: None,
            envs: Vec::new(),
            timeout: None,
        }
    }

    /// `Command::direct(program, args)`: the spec behind
    /// `exec(program, args)` — no shell; `args` is the child's argv as given.
    pub fn direct(program: &str, args: Vec<String>) -> Self {
        Self {
            kind: CommandKind::Direct {
                program: program.to_string(),
                args,
            },
            input: None,
            cwd: None,
            envs: Vec::new(),
            timeout: None,
        }
    }

    /// `Command::input(s)`: pipe `s` to the child's stdin and close it.
    pub fn input(mut self, s: &str) -> Self {
        self.input = Some(s.to_string());
        self
    }

    /// `Command::cwd(path)`: start the child in `path`. Not checked when set —
    /// a directory that does not exist fails the call in the start class.
    pub fn cwd(mut self, path: &str) -> Self {
        self.cwd = Some(path.to_string());
        self
    }

    /// `Command::env(name, value)`: overlay one variable on the inherited
    /// environment. Composes; the later overlay wins on a collision.
    pub fn env(mut self, name: &str, value: &str) -> Self {
        self.envs.push((name.to_string(), value.to_string()));
        self
    }

    /// `Command::timeout(ms)`: budget the child's lifetime from its start.
    pub fn timeout(mut self, ms: u64) -> Self {
        self.timeout = Some(ms);
        self
    }

    /// The shorthand's omitted trailing arguments (`null` in the language).
    fn apply_input(mut self, input: Option<&str>) -> Self {
        self.input = input.map(str::to_string);
        self
    }

    fn apply_timeout(mut self, timeout: Option<u64>) -> Self {
        self.timeout = timeout;
        self
    }

    /// The command's name for diagnostics (informative): the shell line, or
    /// the program `direct` starts.
    fn display(&self) -> String {
        match &self.kind {
            CommandKind::Shell(cmd) => cmd.clone(),
            CommandKind::Direct { program, .. } => program.clone(),
        }
    }

    /// Start the child: stdout/stderr always piped, stdin piped only with an
    /// `input` string (otherwise inherited — an omitted input does not mean
    /// an empty stdin). `cwd` and the `env` overlay apply here, so a bad
    /// directory fails in the start class with the command named.
    fn spawn(&self) -> Result<StdChild, String> {
        let display = self.display();
        let mut cmd = match &self.kind {
            CommandKind::Shell(line) => shell_command(line),
            CommandKind::Direct { program, args } => {
                let mut c = StdCommand::new(program);
                c.args(args);
                c
            }
        };
        cmd.stdin(if self.input.is_some() {
            Stdio::piped()
        } else {
            Stdio::inherit()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        if let Some(dir) = &self.cwd {
            cmd.current_dir(dir);
        }
        for (name, value) in &self.envs {
            cmd.env(name, value);
        }
        cmd.spawn()
            .map_err(|e| format!("cannot run `{display}`: {e}"))
    }

    /// `Command::run()`: start the child and capture both streams to
    /// completion — the one-shot form, budgeted by this spec's `timeout`.
    pub fn run(&self) -> Result<RunOutput, String> {
        capture(self)
    }

    /// `Command::launch()`: start the child and keep the handle; the budget
    /// (if any) rides on the handle, observed when `wait` runs.
    pub fn launch(&self) -> Result<ChildHandle, String> {
        let display = self.display();
        let mut child = self.spawn()?;
        let pid = child.id();
        let out_pipe = child.stdout.take().expect("stdout is piped");
        let err_pipe = child.stderr.take().expect("stderr is piped");
        if let Some(data) = &self.input {
            let pipe = child
                .stdin
                .take()
                .expect("stdin is piped when input is set");
            write_stdin(pipe, data.clone().into_bytes());
        }
        Ok(ChildHandle {
            child: Arc::new(Mutex::new(child)),
            stdout: Some(out_pipe),
            stderr: Some(err_pipe),
            status: None,
            started: Instant::now(),
            timeout: self.timeout,
            spent: false,
            pid,
            display,
        })
    }
}

/// `std::process::run(cmd[, input[, timeout]])`: run `cmd` through the
/// platform shell (`sh -c` on Unix, `cmd /C` on Windows)
/// to completion and capture both streams. `Ok` even when the command fails
/// (a non-zero exit is data); `Err` for a shell that cannot start, an
/// over-cap stream, or an expired budget.
pub fn run(cmd: &str, input: Option<&str>, timeout: Option<u64>) -> Result<RunOutput, String> {
    CommandSpec::shell(cmd)
        .apply_input(input)
        .apply_timeout(timeout)
        .run()
}

/// `std::process::exec(program, args[, input[, timeout]])`: run `program`
/// directly — no shell, `args` becomes the child's argv exactly as given —
/// and capture both streams. The same three `Err` classes as [`run`].
pub fn exec(
    program: &str,
    args: &[String],
    input: Option<&str>,
    timeout: Option<u64>,
) -> Result<RunOutput, String> {
    CommandSpec::direct(program, args.to_vec())
        .apply_input(input)
        .apply_timeout(timeout)
        .run()
}

/// `std::process::launch(cmd[, input[, timeout]])`: start `cmd` through the
/// shell without waiting — `Ok` of the [`ChildHandle`] to collect or stop it
/// later (the `Command::launch` full form reaches the same path with cwd,
/// env overlay, and its own budget).
pub fn launch(cmd: &str, input: Option<&str>, timeout: Option<u64>) -> Result<ChildHandle, String> {
    CommandSpec::shell(cmd)
        .apply_input(input)
        .apply_timeout(timeout)
        .launch()
}

/// A running child behind its handle (spec, "Children"): pipes kept for
/// `wait`, the launch-time budget, and the one-shot discipline — the handle
/// is spent once `wait` answers, `Ok` or `Err` alike, while `pid` keeps
/// answering (the id it was given is a fact).
#[derive(Debug)]
pub struct ChildHandle {
    child: Arc<Mutex<StdChild>>,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    /// A status `kill`'s `try_wait` already observed: killing is not
    /// reaping, but noticing that the child is gone collects its status —
    /// `wait` answers from it instead of waiting again.
    status: Option<ExitStatus>,
    started: Instant,
    timeout: Option<u64>,
    spent: bool,
    pid: u32,
    display: String,
}

impl ChildHandle {
    /// `Child::pid()`: the child's process id — total, spent or not.
    pub fn pid(&self) -> u64 {
        u64::from(self.pid)
    }

    /// `Child::kill()`: deliver a forceful termination. `Err` when there is
    /// nothing to kill — the handle is spent, or the child has already
    /// exited (its status, collected by that check, is still answered by
    /// `wait`, whose `code` is `-1` when the kill was what ended it).
    pub fn kill(&mut self) -> Result<(), String> {
        if self.spent {
            return Err(format!("`{}` child handle is spent", self.display));
        }
        let mut guard = self.child.lock().unwrap();
        match guard.try_wait() {
            Ok(Some(status)) => {
                self.status = Some(status);
                return Err(format!("`{}` child has already exited", self.display));
            }
            Ok(None) => {}
            Err(e) => return Err(format!("cannot kill `{}`: {e}", self.display)),
        }
        guard
            .kill()
            .map_err(|e| format!("cannot kill `{}`: {e}", self.display))?;
        Ok(())
    }

    /// `Child::wait()`: `run` delayed — block, drain both streams while
    /// waiting (so a chatty child cannot wedge on a full pipe), apply the
    /// same 16 MiB cap and lossy decoding, and reap the child. The
    /// launch-time budget is observed here: a child still running past it
    /// is killed and the call yields the timeout `Err`; one that already
    /// finished (even long after its budget) yields its record. Spent after
    /// this returns, with `Ok` or `Err` alike.
    pub fn wait(&mut self) -> Result<RunOutput, String> {
        if self.spent {
            return Err(format!("`{}` child handle is spent", self.display));
        }
        self.spent = true;
        let display = self.display.clone();
        let out_pipe = self.stdout.take().expect("pipes are taken once");
        let err_pipe = self.stderr.take().expect("pipes are taken once");
        let out_h = drain(out_pipe, Arc::clone(&self.child));
        let err_h = drain(err_pipe, Arc::clone(&self.child));
        let status = match self.status.take() {
            Some(status) => Ok(status),
            None => {
                let deadline = self
                    .timeout
                    .map(|ms| self.started + Duration::from_millis(ms));
                wait_until(&self.child, deadline, self.timeout, &display)
            }
        };
        let (out_bytes, out_over) = join_stream(out_h, "stdout", &display)?;
        let (err_bytes, err_over) = join_stream(err_h, "stderr", &display)?;
        let status = status?;
        if out_over || err_over {
            return Err(format!(
                "`{display}` output exceeds {MAX_CAPTURE_BYTES} bytes per stream"
            ));
        }
        Ok(RunOutput {
            stdout: String::from_utf8_lossy(&out_bytes).into_owned(),
            stderr: String::from_utf8_lossy(&err_bytes).into_owned(),
            code: status.code().map(i64::from).unwrap_or(-1),
        })
    }
}

/// The one-shot capture behind `run`/`exec`/`Command::run`: spawn, service
/// every pipe on its own thread, and block until the child exits (or its
/// budget expires).
fn capture(spec: &CommandSpec) -> Result<RunOutput, String> {
    let display = spec.display();
    let mut child = spec.spawn()?;
    let out_pipe = child.stdout.take().expect("stdout is piped");
    let err_pipe = child.stderr.take().expect("stderr is piped");
    let stdin_pipe = child.stdin.take();
    if let Some(data) = &spec.input {
        // Piped stdin: fill it on its own thread and close it so `cat`-style
        // children see end of input. A child that refuses the write by
        // exiting first is not a failure — its status carries the news (the
        // spec's three `Err` classes do not include a stdin write).
        let pipe = stdin_pipe.expect("stdin is piped when input is set");
        write_stdin(pipe, data.clone().into_bytes());
    }
    let child = Arc::new(Mutex::new(child));
    let out_h = drain(out_pipe, Arc::clone(&child));
    let err_h = drain(err_pipe, Arc::clone(&child));
    let deadline = spec
        .timeout
        .map(|ms| Instant::now() + Duration::from_millis(ms));
    let status = wait_until(&child, deadline, spec.timeout, &display);
    let (out_bytes, out_over) = join_stream(out_h, "stdout", &display)?;
    let (err_bytes, err_over) = join_stream(err_h, "stderr", &display)?;
    let status = status?;
    if out_over || err_over {
        return Err(format!(
            "`{display}` output exceeds {MAX_CAPTURE_BYTES} bytes per stream"
        ));
    }
    Ok(RunOutput {
        stdout: String::from_utf8_lossy(&out_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&err_bytes).into_owned(),
        code: status.code().map(i64::from).unwrap_or(-1),
    })
}

/// Drain one piped stream on its own thread with the capture cap: past the
/// cap the reader kills the child first — otherwise a full pipe would leave
/// it blocked on write and the wait would deadlock.
fn drain<R: Read + Send + 'static>(
    pipe: R,
    child: Arc<Mutex<StdChild>>,
) -> std::thread::JoinHandle<(Vec<u8>, bool)> {
    std::thread::spawn(move || {
        let (buf, over) = read_capped(pipe);
        if over {
            let _ = child.lock().unwrap().kill();
        }
        (buf, over)
    })
}

/// Fill the child's stdin from `data` and close the pipe (EOF). Detached:
/// the thread ends when the child's end of the pipe closes, so a child that
/// never reads its input cannot leak it past its own exit.
fn write_stdin(mut pipe: ChildStdin, data: Vec<u8>) {
    drop(std::thread::spawn(move || {
        let _ = pipe.write_all(&data);
        drop(pipe); // EOF: `cat`-style children stop at the end of input
    }));
}

/// Join one reader thread into its captured bytes.
fn join_stream(
    handle: std::thread::JoinHandle<(Vec<u8>, bool)>,
    which: &str,
    display: &str,
) -> Result<(Vec<u8>, bool), String> {
    handle
        .join()
        .map_err(|_| format!("{which} reader of `{display}` panicked"))
}

/// Block until the child exits, enforcing `deadline` (measured from the
/// child's start): past it the child is killed and reaped, and the call
/// fails in the timeout class. Exit is observed before the deadline is
/// checked, so a child that finished — however close to the edge — reports
/// its record. The mutex is only ever held across short operations: the
/// stream readers need it to kill an over-cap child, so a blocking `wait`
/// under the lock could wedge the pipes against them.
fn wait_until(
    child: &Arc<Mutex<StdChild>>,
    deadline: Option<Instant>,
    budget: Option<u64>,
    display: &str,
) -> Result<ExitStatus, String> {
    loop {
        let exited = child
            .lock()
            .unwrap()
            .try_wait()
            .map_err(|e| format!("cannot wait for `{display}`: {e}"))?;
        if let Some(status) = exited {
            return Ok(status);
        }
        if let Some(dl) = deadline
            && Instant::now() >= dl
        {
            let mut guard = child.lock().unwrap();
            let _ = guard.kill();
            let _ = guard.wait();
            return Err(format!(
                "`{display}` timed out after {} ms",
                budget.unwrap_or_default()
            ));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
