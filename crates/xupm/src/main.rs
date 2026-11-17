use std::process::ExitCode;

fn main() -> ExitCode {
    // `xupm run`/`test` host the interpreter, and `build` drives stage 1 through
    // it, so `xupm` needs the same oversized stack as `xulo`: Windows gives the
    // main thread only 1 MiB, and the bootstrap workload exceeds that.
    xulo_cli::run_on_bootstrap_stack(xupm::cli::run)
}
