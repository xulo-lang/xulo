# Changelog

All notable changes to Xulo are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Aligned with `spec@815edad` (specification 0.2.0), which removes the component
layer from the language.

### Removed

- The component/UI language layer: `View`, `@State`, `@Store`, `@Effect`,
  `@Environment`, component declarations and blocks, `$` binding arguments, and
  the `@` token.
- `main` must now return `unit`; `fn main(): View` is rejected.
- Diagnostic codes `E0701`–`E0799` (reserved and never reused).
- The UI crates (`xulo-framework`, `xulo-ui`, `xulo-ui-wasm`,
  `xulo-render-core`, `xulo-renderer-terminal`, `xulo-renderer-skia`,
  `xulo-renderer-webview`) and the CLI `--render`, `webview`, and `skia`
  options/features.

UI construction remains possible with ordinary `struct`, `enum`, and `fn`
values, and is left to a future standard-library `std/ui` module.

### Changed

- The `README` and `learn/book` guide now describe the UI-less 0.2.0 language:
  `mod` / `use` / `pub use` instead of `import ... from`, `Result<T, E>` and `?`
  instead of `try` / `catch` / `throw`, the specification's 61-keyword and
  reserved-word tables, `int`/`float`/`number` distinctions, and the current
  stage 1 self-hosting status.
- The formatter/editor token layer now classifies exactly the specification's
  closed keyword set and reserved list (spec `lexical-structure.md`):
  `print`/`println` are ordinary prelude names, `try`/`catch`/`throw`/
  `import`/`from` lex as identifiers, and `defer`/`move`/`self`/`spawn`/
  `super` are keywords rather than reserved words.

### Added

- Struct deconstruction in `match` (`Point(x, y)`) now works on all three paths:
  the checker validates the struct type and field count and binds each field
  with its declared type, the interpreter deconstructs the object by
  declaration order, and the AOT backend reads the fields through `ObjectGet`.
  The grammar gate's last `error-only:` row (`examples/coverage.tsv`,
  `StructPattern`) now points at `examples/types/structs.xulo`. Nested patterns
  inside a struct slot (`Shape::Dot(Point(x, y))`) remain unsupported and are
  rejected with a clear diagnostic.
- `examples/std/` now covers the 38 standard-library members the grammar gate
  could not see: `math_extra.xulo` (trig/log/`trunc`/`clamp`/`sign`, the integer
  bit ops, `random`), `fs_extra.xulo` (temporary creators, exclusive create,
  byte append, times/mode, `realpath`), `io_extra.xulo` (`eprint`, the tty
  probes, the EOF reads), `net_shutdown.xulo` (`sock_shutdown`), `exit` in
  `process.xulo`, and a `split` example in `regex.xulo`. Every one builds
  natively and prints what the interpreter prints.
- `crates/xulo-cli/tests/examples.rs::std_members_are_exercised_by_the_corpus`
  keeps that coverage honest: it walks every row of
  `std-spec/signature-index.md`, ignores comments, exempts `type …` rows, and
  fails when a callable member has no `std::<module>::<member>` call (or
  `.method(` for the `string.*` pseudo members). It skips when the `std-spec`
  submodule is not checked out.

### Fixed

- AOT: `+` on a string or list bound from an enum payload no longer goes
  through the dynamic-number path. `match std::fs::read(p) { Result::Ok(dir) =>`
  used to hand a float to `xulo_string_concat`, so `dir + "/file"` crashed (or
  concatenated two raw pointers when both operands were payload bindings).
  Regression:
  `crates/xulo-compiler/tests/codegen_tests.rs::test_codegen_string_concat_with_match_payload_binding`
  and `::test_codegen_list_concat_with_match_payload_binding`.
- AOT: user functions now emit as `xulo_fn_<name>`, so a program may define
  `fn read(…)`, `fn listen(…)`, or any other name that libc/Winsock also uses
  without the link failing with `LNK2005: listen already defined`. The entry
  point stays `main` and `pub extern "C" fn` exports keep their declared name.
  Regression: `crates/xulo-compiler/tests/aot_symbols.rs`.
- AOT: `string.split` with a compiled `RegExp` was lowered as a string
  separator, so `"ada, bob".split(re)` split into characters. The intrinsic now
  carries the argument's tag and dispatches to `RegExp::split` for a regex
  payload. Regression:
  `crates/xulo-compiler/tests/regex_codegen_tests.rs::string_split_follows_a_compiled_regexp`
  and `::string_split_accepts_a_separator_string`.

## [0.1.0] - 2026-11-14

First public release, stage 0: the Rust bootstrap toolchain.

### Added

- Native Rust interpreter (`xulo run`) for the core language: literals,
  variables, functions, closures, recursion, control flow, `match`, enums,
  lists, objects, `try`/`catch`, optional chaining, null coalescing, spread,
  `print`/`println`/`str`, `async`/`await`, local `import`/`pub`, and UI
  component syntax.
- CLI (`xulo`) with `run`, `check`, `fmt`, and `repl`.
- `xupm` package manager with native AOT builds through the LLVM 22 backend
  (statically linked LLVM in distribution builds, Cranelift fallback when the
  `llvm` feature is disabled).
- LSP language server (`xulo-analyzer`) with diagnostics, go-to-definition,
  hover, references, outline, formatting, and semantic highlighting.
- Terminal renderer for `main(): View` (`xulo run --render terminal`).
- Language and standard-library specifications in the `spec` and `std-spec`
  submodules (version 0.1.0).
- `install.sh` and a `curl | sh` install path for GitHub Releases, plus
  `install.ps1` for Windows PowerShell.
- Multi-platform release artifacts: Linux x86_64, macOS Intel/Apple Silicon,
  and Windows x86_64.
- MIT license.

### Changed

- Removed the UI examples (`examples/ui.xulo`, `examples/layout.xulo`) and the
  `@xulo/ui` demo shim; the UI language surface and renderers remain available.

### Known limitations

- The Linux release builds `xupm` with statically linked LLVM 22. The macOS
  and Windows stage-0 previews build `xupm` with the Cranelift backend instead
  (no LLVM installation required); install LLVM 22 and build with
  `--features distribution` for the product AOT path on those hosts.
- The interpreter's FFI (`extern "C"`) lookup is Unix-only; Windows returns a
  clear "unsupported" error. AOT FFI is wired for the Windows calling
  convention but is not yet covered by CI.
- The webview and Skia renderers are not part of this release artifact; the
  webview backend also requires the `wasm32-unknown-unknown` target at build
  time.
- The formatter discards comments.
- `@Environment` is not implemented.
- The compiler is a tree-walking interpreter plus an LLVM/Cranelift AOT path;
  self-hosting is not part of stage 0.

[0.1.0]: https://github.com/xulo-lang/xulo/releases/tag/v0.1.0
