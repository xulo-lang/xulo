use std::process::ExitCode;

fn main() -> ExitCode {
    // The interpreter and stage 1's recursive-descent parser are deeper than
    // the default Windows main-thread stack (1 MiB). Run the CLI on a thread
    // with the same large stack the Unix main thread gets, so recursive
    // bootstrap programs behave identically on every host.
    std::thread::Builder::new()
        .name("xulo".to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(xulo_cli::cli::run)
        .expect("cannot spawn the xulo runtime thread")
        .join()
        .unwrap_or(ExitCode::FAILURE)
}
