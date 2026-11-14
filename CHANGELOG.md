# Changelog

All notable changes to Xulo are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
