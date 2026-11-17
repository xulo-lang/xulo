use std::process::ExitCode;

fn main() -> ExitCode {
    // The interpreter and stage 1's recursive-descent parser are deeper than
    // the default Windows main-thread stack (1 MiB). Run the CLI on a thread
    // with the same large stack the Unix main thread gets, so recursive
    // bootstrap programs behave identically on every host.
    xulo_cli::run_on_bootstrap_stack(xulo_cli::cli::run)
}
