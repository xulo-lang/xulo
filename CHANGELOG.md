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

- The AOT differential now accepts `@aot-failure-shape <platform>:` (T13 option
  c). Examples that declare it must fail on both paths with the same normalized
  runtime message; stdout and diagnostic decoration are not compared, so the
  interpreter keeps its `error[E0006]` + source snippet and the native binary
  keeps `panic: …`. `errors/division_by_zero.xulo` and
  `errors/runtime_failure.xulo` use the directive instead of `@no-aot`.
  Regression:
  `crates/xupm/tests/aot_differential.rs::failure_shape_normalizes_both_renderings`.
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

- Stage 1's C backend lowers postfix `?` (propagating `Result::Err` with a
  `return`), `Error("…")`, assignment in expression position (`a = b = 3`),
  `shared` as a pass-through unary, and `Array::filled<T, N>` with a literal
  length. Call-site type arguments are dropped because the backend is
  dynamically typed, and Xulo's `\u{…}` escapes are translated to C's
  `\uXXXX`/`\UXXXXXXXX` form. The bootstrap runtime now expands `%g` exponent
  notation to plain decimal, so `1000.0` prints like stage 0. The stage 1
  build coverage test pins `basics/lexical.xulo`, `errors/propagation.xulo`,
  and `collections/arrays.xulo` end to end.
- Stage 1's parser now accepts the whole example corpus, so
  `crates/xulo-cli/tests/stage1_examples_baseline.tsv` is empty (66 files, no
  lexer or parser gaps). It gained postfix `?` with the specification's
  ternary/postfix disambiguation, arrow closures, call-site type arguments,
  literal types, tuple position chains (`p.0.1`), right-associative chained
  assignment, function-macro calls (bodies are skipped as token trees), `lock`,
  `spawn`, and service spawn. Binary operators were rewritten as a
  precedence-climbing loop, which keeps deeply nested `match` expressions
  (`std/net.xulo`, `std/http.xulo`) under the interpreter's 128-call limit.
  Regressions: `xuloc/src/tests.xulo::parses_postfix_propagation_beside_ternary`
  and the other parser tests there;
  `crates/xulo-cli/tests/stage1.rs::stage1_example_corpus_tracks_baseline`.
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

- The conformance runner no longer passes rejected examples vacuously. It now
  strips an inline `// error[E…]` annotation from the staged source, keeps a
  fully-commented statement's trailing annotation as a comment, inserts a
  rejected line at the position left after earlier rejected lines are removed,
  and reads codes only from diagnostic headers instead of echoed snippets. The
  fix cleared 6 cases, exposed 19 masked divergences (now triaged in
  `spec_conformance_baseline.tsv`: 46 known, 65 of 111 pass), and the parser now
  accepts the pinned examples' prefix parameter mode (`fn watch(mut xs: …)`)
  and a brace map's string key (`{ "a": 1 }`). A default integer literal above
  `u64::MAX` is now `E0209`. Regression:
  `crates/xulo-cli/tests/spec_conformance.rs` (three runner tests) and
  `crates/xulo-semantic/tests/semantic_tests.rs::default_integer_literal_above_u64_max_is_e0209`.
- Stage 1's C backend now matches the stage 0 runtime on the last four
  drift-governance deviations (D-01–D-04). Integral `+`/`-`/`*`, `/`, `%`,
  `**`, and unary `-` are checked and fail with `panic: integer overflow`
  instead of wrapping or hitting C UB; runtime failures print stage 0's
  `panic: <message>` prefix; floats keep the plain-decimal shortest form
  (`1e300`, `1e-300`, `1e21`); `u64` annotations and integer literals above
  `i64::MAX` are rejected with `E0005` instead of being mis-tagged as `int`.
  Windows C builds also link `advapi32` explicitly (the runtime's
  `GetUserNameA` call previously depended on the host linker's defaults), and
  the Windows runtime includes `<process.h>` for `_getpid`.
  Regression:
  `crates/xupm/tests/stage1_build.rs::stage1_c_backend_traps_like_the_stage0_runtime`,
  `::stage1_c_backend_matches_stage0_float_formatting`, and
  `::stage1_rejects_u64_in_the_bootstrap_subset`.
- Windows `extern "C"` FFI now works end to end. The interpreter resolves a
  symbol through the process's loaded modules (`GetProcAddress`, falling back to
  the executable and UCRT) instead of returning an "unsupported" error; the JIT
  already resolved through cranelift-jit's Windows lookup, and AOT leaves the
  import to the linker so it binds the UCRT. `examples/ffi/ffi.xulo` no longer
  carries `@no-aot windows` / `@no-run windows`, so the AOT differential covers
  it on every host. Regression: `crates/xulo-runtime/tests/ffi_tests.rs` and
  `crates/xulo-compiler/tests/ffi_codegen_tests.rs` (both un-gated from Unix).
- Both engines now enforce the `f32` format (T12). The interpreter carries
  `Value::Float32`/`Num::Float32` and rounds binding annotations, parameters,
  returns, and every f32 arithmetic result; the AOT backend rounds f32
  bindings, parameters, returns, struct fields, and f32 operations through
  `xulo_round_f32`. `let o: f32 = 0.8` now prints
  `0.800000011920929` on both paths, and `1.0 / 3.0` in f32 prints
  `0.3333333432674408`. `types/float_math.xulo` covers values that differ
  between binary32 and binary64. Regression:
  `crates/xulo-runtime/tests/type_system.rs::f32_rounds_at_binding_and_arithmetic`
  and
  `crates/xulo-compiler/tests/codegen_tests.rs::test_codegen_fixed_width_float_annotations_print_as_floats`.
- AOT: `spawn` / `lock` / service now lower. The compiled concurrency model is
  synchronous: a spawn block runs immediately (so `await` is the identity),
  `lock name { … }` evaluates its block, `spawn.process Type { … }` builds a
  deep-copied service state and calls the compiled `on_message` on `send`, and
  `reply(v)` records the value `send` returns. This matches the interpreter's
  eager async model for the checked forms. `concurrency/spawn_lock.xulo` no
  longer carries `@no-aot`. Regression:
  `crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs::spawn_lock_and_services_follow_the_compiled_model`.
- AOT: dynamic trait dispatch through a generic parameter now works. Struct
  construction registers the source type name alongside the value-category
  marker (and copies preserve it), and `dyn:{Trait}:{method}` picks the
  registered `impl_{Trait}_{Type}_{method}` by testing the receiver's runtime
  struct/enum name (primitive impls use the runtime tag). This covers
  `Area::area(x)` with `x: T`, generic `impl`s such as `Box2<T>`, and inherent
  methods that loop over a `List<T>`. `types/generics_impl.xulo` no longer
  carries `@no-aot`. Regression:
  `crates/xulo-compiler/tests/trait_dispatch_codegen_tests.rs::generic_trait_dispatch_runs`.
- AOT: match guards, or-patterns, numeric range patterns, and type patterns now
  lower instead of being rejected. Guards run after the pattern's bindings; an
  or-pattern tests each alternative before one shared arm body; ranges compare
  numbers (and strings) with the same semantics as the interpreter; type
  patterns test the runtime tag and enum type name. Direct calls now pass the
  runtime tags of dynamic (`unknown`/unannotated/union/optional) arguments, so
  `bool`/`int` patterns distinguish values that share the `0`/`1` slot.
  `patterns/match_patterns.xulo` no longer carries `@no-aot`. Regression:
  `crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs::match_guards_or_ranges_and_type_patterns`.
- AOT: `defer` now lowers to a per-block queue instead of being rejected. The
  operand runs in reverse registration order when its block exits, including
  before `return` and `?` propagation, and on loop `break`/`continue`; nested
  blocks, closures, and match arm blocks drain their own queues.
  `functions/defer.xulo` no longer carries `@no-aot`. Regression:
  `crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs::defer_runs_at_block_exit_in_reverse`
  and `::defer_runs_on_break_continue_and_return`.
- AOT: nested tuple positional access (`nested.0.0`) now resolves the element
  type through every level, and a tuple returned from or passed to a function
  keeps its array tag instead of printing as a raw pointer. `types/tuples.xulo`
  no longer carries `@no-aot`. Regression:
  `crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs::nested_tuple_access_and_tuple_pass_return`.
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
