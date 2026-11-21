//! Sidecars must not record the checkout path: the examples gate runs on
//! Linux, macOS, and Windows, whose absolute paths all differ.

/// `portable` rewrites the canonical entry path to `examples/…` with forward
/// slashes, which is what a committed `.stderr.expected` may contain.
#[test]
fn absolute_entry_paths_become_repository_relative() {
    let entry = xulo_examples::examples_dir().join("errors/division_by_zero.xulo");
    let canonical = std::fs::canonicalize(&entry).expect("the example exists");
    let text = format!(" --> {}:15:11\n", canonical.display());

    let out = xulo_examples::portable(&entry, &text);

    assert_eq!(out, " --> examples/errors/division_by_zero.xulo:15:11\n");
}

/// Output that does not mention the entry path is left untouched.
#[test]
fn unrelated_output_is_unchanged() {
    let entry = xulo_examples::examples_dir().join("errors/runtime_failure.xulo");
    assert_eq!(
        xulo_examples::portable(&entry, "error[E0006] runtime: boom\n"),
        "error[E0006] runtime: boom\n"
    );
}
