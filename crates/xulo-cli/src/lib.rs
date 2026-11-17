pub mod cli;

use std::process::ExitCode;

/// Stack reservation for entry points that host the interpreter and stage 1's
/// recursive-descent parser.
///
/// Unix gives a process's main thread 8 MiB, but Windows gives it 1 MiB, and
/// the bootstrap workload (`xuloc/tests.xulo` parsing its own sources) recurses
/// well past that. The native backends reserve the same amount (`link.exe`
/// `/STACK`, or the platform default on Unix), so every entry point that runs
/// Xulo code requests it explicitly instead of inheriting the host default.
pub const BOOTSTRAP_STACK_SIZE: usize = 64 * 1024 * 1024;

/// Run `run` on a thread with [`BOOTSTRAP_STACK_SIZE`] and return its exit
/// code. Used by `xulo` and `xupm` so both behave the same on every host.
pub fn run_on_bootstrap_stack<F>(run: F) -> ExitCode
where
    F: FnOnce() -> ExitCode + Send + 'static,
{
    std::thread::Builder::new()
        .name("xulo".to_string())
        .stack_size(BOOTSTRAP_STACK_SIZE)
        .spawn(run)
        .expect("cannot spawn the xulo runtime thread")
        .join()
        .unwrap_or(ExitCode::FAILURE)
}
