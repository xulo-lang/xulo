# Stage 1 Bootstrap Tasks

- **Status**: Living checklist
- **Date**: 2026-11-17
- **Companion document**: [`stage1-readiness.md`](stage1-readiness.md) — rationale, evidence, and gate definitions
- **Normative sources**: `spec@815edad` and `std-spec@a2e4cb7`, as pinned by this repository
- **Scope**: the executable checklist for making stage 0 host-ready and bootstrapping stage 1

This file is the execution view of `stage1-readiness.md`. The readiness document
explains *why* each item matters and records the evidence; this file only tracks
*whether it is done*. IDs (`L2`, `R9`, `T3`, …) match the matrix in the
readiness document.

## How to use this checklist

- `[x]` means done and verified by a test or a named probe.
- `[ ]` means not done, not verified, or currently broken.
- Do not check an item because the code "looks right". Check it when its
  verification command passes and the proof lives under a `tests/` directory.
- When an item changes state, update the matching row in
  `stage1-readiness.md` in the same change.
- Never check a later gate while an earlier gate is still open.

## Progress

| Work package | Done | Total |
|---|---:|---:|
| WP0 — Matrix hygiene and conformance | 5 | 5 |
| WP1 — 64-bit integer model | 7 | 7 |
| WP2 — Type system and semantic convergence | 12 | 12 |
| WP3 — AOT parity and bootstrap backend | 13 | 13 |
| WP4 — `defer` semantics | 5 | 5 |
| WP5 — Stage 1 skeleton | 6 | 6 |
| WP6 — Self-host loop | 7 | 7 |
| WP7 — Non-gating follow-ups | 9 | 10 |
| **Total** | **64** | **65** |

## Gates

Gates are cumulative. An open earlier gate blocks every later gate.

- [x] **Gate 1 — Interpreter prototype**: stage 1 lexer/parser runs under
      `xulo run`, reads a `.xulo` file, and prints tokens/AST or a clean
      diagnostic. `crates/xulo-cli/tests/stage1.rs`.
- [x] **Gate 2 — Self-parse**: stage 1 parses its own source (or a fixed
      nontrivial bootstrap file) with accurate spans.
      `crates/xulo-cli/tests/stage1.rs` parses `xuloc/*.xulo` itself.
- [x] **Gate 3 — Self-check**: stage 1 type-checks a nontrivial bootstrap file
      and agrees with stage 0 on accept/reject; diagnostics carry
      file/line/column and exit nonzero.
      `xuloc/checker.xulo` + `xuloc/corpus/`; verified by
      `crates/xulo-cli/tests/stage1.rs::stage1_check_agrees_with_stage0_on_the_corpus`.
- [x] **Gate 4 — Native build**: `xupm build` produces a stage 1 binary whose
      output matches `xulo run` on the bootstrap workload.
      `crates/xupm/tests/aot_differential.rs::stage1_binary_matches_the_interpreter`.
      Reached on Linux, macOS, and Windows (macOS verified locally on
      2026-11-17; the Windows differential and stage 1 build pass locally and
      in CI, closing B7 in WP3).
- [x] **Gate 5 — Self-host**: stage 1 compiles itself into stage 2; stage 1 and
      stage 2 agree on the bootstrap suite and the conformance subset.
      `crates/xupm/tests/stage1_build.rs::stage1_builds_itself_into_stage2` and
      `::stage1_and_stage2_agree_on_the_bootstrap_suite` (tokens/parse of every
      stage 1 source, check verdicts and diagnostics for the accept/reject
      corpus, and the corpus programs' runtime output).
- [x] **Gate 6 — Conformance**: the pinned specification's checkable examples
      and the bootstrap suite run in CI. The `conformance` job runs the
      pinned-spec runner, the stage 1 suite, and the opt-in self-host build
      (`XULO_STAGE1_BUILD=1`); deviations stay triaged in
      `spec_conformance_baseline.tsv` (§6.10).

## Baseline capabilities

Capabilities stage 1 may rely on today. These are not work packages; they are
the starting point, and their evidence is recorded in `stage1-readiness.md` §5.
Checked entries are OK. Unchecked entries are PARTIAL; each names the work
package or out-of-scope section that tracks it.

- [x] **L1** UTF-8 lexing and parsing of `.xulo` files
- [x] **L3** `float` / `f64` arithmetic
- [x] **L4** `let`, `let mut`, assignment, shadowing
- [x] **L5** Functions, recursion, implicit return
- [x] **L6** Closures and function values
- [x] **L7** `if` / `else`, `while`, `for`, `match`, `break`, `continue`
- [x] **L8** Enums, payloads, recursive/generic enums, exhaustive `match`
- [x] **L9** Structs, `impl`, traits, explicit `Trait::method(recv)` dispatch
- [x] **L10** Generic functions, bounds, `where` clauses
- [x] **L12** `mod` / `use` / `pub`, visibility, cross-file modules
- [x] **L13** Tuple literals, positional access, destructuring, and structural
      equality for `List`/tuple/`struct` (`stage1-readiness.md` §6.11).
- [x] **L16** `T?`, `?.`, `??`, null narrowing
- [x] **L19** `Result` / `Error` and `?` propagation
- [x] **L21** `copy` / `move` and use-after-move checking (outside the bootstrap
      subset; WP7)
- [x] **L22** Async/await, `spawn`, `lock`, `shared` (outside the bootstrap subset)
- [x] **L23** Attributes, `#[test]`, `#[derive]`, macros (outside the bootstrap subset)
- [x] **L25** Type aliases (`type X = …`), including alias receivers, dispatch, and function-type aliases
- [x] **R1** Deterministic left-to-right evaluation
- [x] **R3** UTF-8 scalar string operations
- [x] **R4** Runtime failures with stable codes
- [x] **R5** No panic on adversarial input; nesting guard
- [x] **R6** `panic`, `assert`, and process exit status
- [x] **R8** Value/reference semantics for `copy` / `move`: `copy` deep-copies,
      `move` transfers, and plain `let`/assignment/loop/match binding copies
      value-category aggregates (`struct`, `enum`, `Array`, tuple, `Range`)
      while `List`/`Map`/`Set`/`StringBuilder` stay shared
      (`crates/xulo-runtime/tests/value_semantics.rs`).
- [x] **R9** Interpreter and AOT produce identical observable behavior — all 30
      top-level examples plus the stage 1 workload match byte for byte
      (WP3 R9c)
- [x] **R10** Deterministic formatting of integers and diagnostics
- [x] **C4** `Array<T, N>` with `.len`, indexing, iteration, and `Array::filled`
- [x] **C5** `string` members and UTF-8 behavior
- [x] **C6** `StringBuilder` with `new` / `append` / `clear` / `build` / `len` / `byte_len`
- [x] **C7** `str` / `fmt` / `print` output for every bootstrap value
- [x] **S1** `std::fs`
- [x] **S2** `std::io`
- [x] **S3** `std::process`
- [x] **S4** `std::path`, `std::os`, environment, cwd, pid
- [x] **S5** `std::json`
- [x] **S6** `std::regex` and `std::fmt`
- [x] **S7** `std::net` and `std::http` (outside the bootstrap subset)
- [ ] **S8** `extern "C"` FFI — PARTIAL: Unix only, Windows unsupported; outside
      the bootstrap subset (out of scope, below)
- [x] **S10** `std::math` (interpreter and AOT)
- [x] **S11** `std::time` (interpreter and AOT)
- [x] **B1** `xulo run` and `xulo check` for a single file
- [x] **B2** `xupm build` for core language and `hello`
- [x] **B3** `xupm build` for the bootstrap subset
- [x] **B4** AOT module tree (`mod` / `use`)
- [x] **B5** Runtime static library and linker path
- [x] **B7** Linux/macOS/Windows build matrix — Linux and Windows run the full
      AOT differential in CI, and macOS is verified locally; Windows parks
      `ffi.xulo` (Unix-only FFI, S8) and passes locally and in CI on 2026-11-17
- [x] **B8** Reproducible stage 1 build and self-compile smoke test — B8a builds
      the native front end; B8b in WP6 shows two stage 1 builds emit identical
      C and that stage 2 regenerates exactly the translation unit that built it
      (`crates/xupm/tests/stage1_build.rs::stage1_build_is_reproducible`)
- [x] **D1** Source spans and `file:line:column` diagnostics
- [x] **D2** stderr writes and nonzero exit status
- [x] **D4** Stage 1 diagnostics consistent with stage 0
- [x] **T1** Unit and integration tests — `cargo test --workspace` is green; the
      count grows with the corpus, so record it from CI rather than here
- [x] **T2** All examples run in CI — `crates/xulo-cli/tests/examples.rs`
      discovers every program unit, runs `xulo check`, executes it, and compares
      stdout/stderr with the `<name>.expected` sidecar; the CI `examples` job
      drives it with the release binary
- [x] **T5** Stage 1 self-parse/self-check/self-build tests — self-parse,
      self-check, the self-build (stage 2), and the stage 1/stage 2
      differential live in `crates/xulo-cli/tests/stage1.rs`,
      `xuloc/tests.xulo`, and `crates/xupm/tests/stage1_build.rs`
- [x] **T6** Robustness/fuzz tests

## WP0 — Matrix hygiene and conformance

Priority P0 · needed by Gate 1/2 · source: `stage1-readiness.md` §7 WP0, §6.9

- [x] **T7** Add a CI check that `git submodule status` matches the pinned
      `spec@815edad` and `std-spec@a2e4cb7` commits.
      `crates/xulo-cli/tests/submodule_pins.rs`.
- [x] **T3a** Add a conformance runner that extracts `// ok` blocks from the
      pinned specification and runs them through stage 0.
      `crates/xulo-cli/tests/spec_conformance.rs` (`Accept` cases).
- [x] **T3b** Extend the runner to `// error[E…]` blocks with expected codes.
      Each rejected line is re-inserted uncommented and must produce its code.
- [x] **T3d** Ensure every row marked OK in `stage1-readiness.md` is backed by
      a test or a named probe; add the missing ones.
      `crates/xulo-cli/tests/readiness_matrix.rs` requires every OK row to cite
      an existing path, runs the `crates/xulo-cli/tests/matrix_probes/` corpus,
      and fails on a probe that no row cites. The audit found and fixed two
      defects (a trailing struct construction lost its implicit return; tuple
      destructuring rejected tuple types) and
      re-graded L13 to PARTIAL for identity-based equality (§6.11, since resolved by L13b).
- [x] **T3e** Add the conformance job to `.github/workflows/ci.yml` and triage
      its failures in the readiness matrix.
      Job `conformance`; the current divergences are listed with a reason in
      `crates/xulo-cli/tests/spec_conformance_baseline.tsv` and summarised in
      `stage1-readiness.md` §6.10.

**Verify:** `cargo test -p xulo-cli --test submodule_pins --test spec_conformance --test readiness_matrix`.

## WP1 — 64-bit integer model

Priority P0 · needed by Gate 1/2/5 · source: `stage1-readiness.md` §7 WP1, §6.1

- [x] **L2a** Split integer and floating literals in `xulo-core`
      (`Literal::Int(i128)` + `Literal::Float(f64)`).
- [x] **R2a** Split integer and floating runtime values in `xulo-runtime`
      (`Value::Int(i64)` / `UInt(u64)` / `Float(f64)`).
- [x] **R2b** Update arithmetic, comparison, formatting, and FFI for the new
      integer representation. AOT lowers integral operators to checked
      `xulo_int_*` helpers; `xulo_num_compare` compares across signedness;
      `u64` prints with tag `10`.
- [x] **L2b** Parse `u64` literals without an `f64` round trip; accept
      `18446744073709551615`. A literal above `u64::MAX` is `E0209`.
- [x] **R9a** Make the interpreter and AOT agree on overflow, wrapping, shifts,
      and integer printing (`crates/xulo-runtime/tests/integers.rs`,
      `crates/xulo-compiler/tests/integer_model_codegen_tests.rs`).
- [x] **R2c** Add edge tests: `2^53`, `2^63`, `2^64-1`, `0 - 1`, `1 << 63`,
      `to_fixed`, and the wrapping intrinsics.
- [x] **G1** Make the `§6.1` probe pass under both `xulo run` and
      `xupm build`.

**Verify:** `9007199254740993 == 9007199254740992` is `false`; `1 << 62` is
`4611686018427387904`; the `u64` max literal is accepted.

## WP2 — Type system and semantic convergence

Priority P0 · needed by Gate 2/3 · source: `stage1-readiness.md` §7 WP2, §6.2–6.5

- [x] **L17a** Implement `unknown` as the top type, including `List<unknown>`
      and `Map<string, unknown>` (`assignable` treats `unknown` as top).
- [x] **L17b** Accept `unknown` in parameter and return positions.
- [x] **L18a** Implement `is` / `is not` branch narrowing as specified in
      `expressions/control-flow.md` (branch types, union complement, E0222).
- [x] **L18b** End chronological narrowing on assignment.
- [x] **L14a** Implement tuple positional access `p.0` / `p?.0`.
- [x] **L14b** Support tuple positional writes through `let mut`.
- [x] **L11** Implement generic struct construction from named arguments and
      the expected type.
- [x] **D3** Provide stable error codes for the bootstrap subset instead of
      coarse fallbacks.
- [x] **S9** Align std numeric signatures and diagnostics with `std-spec` §1.3.
- [x] **G2** Add regression tests lifted from the pinned specification and make
      the `§6.2–6.5` probes pass
      (`crates/xulo-runtime/tests/type_system.rs`,
      `crates/xulo-compiler/tests/type_system_codegen_tests.rs`).
- [x] **L6** Closures: an untyped lambda passed to a generic callback no longer
      fails with `E0202`; its provisional `any` parameter type is a unification
      wildcard, so a later argument fixes the type parameter.
      `crates/xulo-semantic/tests/semantic_tests.rs::untyped_closure_infers_generic_callback_type`,
      `crates/xulo-compiler/tests/type_system_codegen_tests.rs::generic_closure_argument_compiles`.
- [x] **L13b** Structural `==` for `List`, tuple, and `struct` values (and
      `Map`/`Set`/enums), per `stage1-readiness.md` §6.11. The runtime
      `xulo_value_equal` helper recurses through aggregates with a visited-pair
      guard, and the AOT path routes structural `==`/`!=` through it.
      `crates/xulo-runtime/tests/equality.rs`,
      `crates/xulo-compiler/tests/type_system_codegen_tests.rs::structural_equality_compiles`.

**Verify:** the four probes in `stage1-readiness.md` §6.2–6.5 run without a
workaround.

## WP3 — AOT parity and bootstrap backend

Priority P1 · needed by Gate 4 · source: `stage1-readiness.md` §7 WP3, §6.6–6.7

- [x] **C2** Add AOT runtime support for `Map`.
- [x] **C3** Add AOT runtime support for `Set`.
- [x] **C1** Fix list spread lowering in the AOT path
      (`examples/collections/collections.xulo`).
- [x] **R9b** Fix nested ternary lowering in the AOT path
      (`examples/types/optional_chaining.xulo`).
- [x] **L15** Fix `Array<T, N>.len` in the AOT path.
- [x] **T4** Add a differential runner that builds every example program both
      ways and compares output. Discovery and the `@no-aot` / `@skip-aot` /
      `@no-run` directives come from `crates/xulo-examples`, shared with the
      `examples` gate, and a `@no-aot` marker that starts building fails the run.
- [x] **B3a** Verify that `xupm build` failures return a nonzero exit status;
      fix the pipeline if they do not.
- [x] **G3** Make the `§6.6–6.7` probes pass.
- [x] **B3c** AOT lowering for `std::fs::write_bytes`/`read_bytes` (`fs`),
      `std::path::split`/`relative` (`path`), and the `std::http` server types
      (`listen`, `Server::serve/addr`, `Request`/`Response` constructors and
      lookups). `fs.xulo` and `path.xulo` are out of the differential baseline;
      `http.xulo` compares byte for byte once `S3a` and the await lowering land.
      Regression: `crates/xulo-compiler/tests/type_system_codegen_tests.rs::http_data_types_compile`.
- [x] **S3a** AOT lowering for `std::process::launch`, `Command`, and `Child`
      (`process`). `crates/xupm/tests/aot_differential.rs` now compares
      `examples/std/process.xulo` byte for byte.
- [x] **R9c** `async_tasks`: AOT lowers `Task::resolve`/`all`/`race` to their
      already-settled values and `Task::timeout` to an elapsed-time check
      against the limit; `examples/concurrency/async_tasks.xulo` now matches `xulo run`
      byte for byte. Regression:
      `crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs::task_combinators_follow_the_compiled_async_model`.
      Outside the bootstrap subset (§8 of `stage1-readiness.md`).
- [x] **C4** Verify `Array::filled` under both execution paths.
      `crates/xulo-runtime/tests/native.rs::array_filled_runs` (interpreter) and
      `crates/xulo-compiler/tests/type_system_codegen_tests.rs::array_filled_compiles`
      (AOT runtime lowering `xulo_array_filled`).
- [x] **B7** Run `stage1_binary_matches_the_interpreter` on macOS and Windows,
      not only Linux. macOS is verified: the opt-in differential compares the
      example corpus and the stage 1 workload byte for byte on 2026-11-17, and CI
      runs it there. Windows runs the same differential; `ffi.xulo` declares
      `// @no-aot windows:` because `extern "C"` FFI stays Unix-only (S8). The
      Windows link now names the Rust runtime's system libraries
      (`kernel32`/`ntdll`/`userenv`/`ws2_32`/`dbghelp`/`synchronization` plus
      `/DEFAULTLIB:msvcrt`), and `examples/std/fs.xulo` no longer prints its host
      temp path, so the comparable examples and the stage 1 native build pass
      byte for byte locally and in CI on 2026-11-17. Windows FFI stays out of
      scope (S8).

**Verify:** `xupm build examples/collections/collections.xulo` and
`xupm build examples/types/optional_chaining.xulo` match `xulo run` byte for byte.
`XULO_AOT_DIFFERENTIAL=1 cargo test -p xupm --test aot_differential` compares
every discovered program.

## WP3b — Example corpus as the syntax gate

Priority P1 · needed by Gate 3 and the bootstrap corpus · source:
`stage1-readiness.md` §5 T8/T9, §6.12

- [x] **T8** Make `examples/` cover the whole grammar, and prove it.
      `examples/coverage.tsv` maps all 194 nonterminals of `spec/grammar.md`'s
      production index to an example plus a probe string;
      `crates/xulo-cli/tests/examples.rs::coverage_table_accounts_for_grammar`
      fails on a missing row, a stale path, an absent probe, or an exemption
      without a reason. Discovered while building the corpus:
      - `spawn.on(expr)` never parsed (`spawn_target` assumed the expression was
        the first child of the pair, but `kw_on` is a non-silent token) — fixed in
        `crates/xulo-parser/src/expr.rs`, regression
        `spawn_on_target_keeps_its_expression`;
      - `mut` parameters were declared `is_const`, so assigning to one was
        rejected as "binding is immutable" — fixed in `crates/xulo-semantic`;
      - `StructPattern` is parsed but unimplemented (§6.12), recorded as the one
        `error-only:` row; implemented later on all three paths (checker binds
        the fields, the interpreter and AOT deconstruct the object), so the row
        now points at `types/structs.xulo`;
      - `PathHead` is covered by both heads: `modules/inline/main.xulo` uses
        `use self::inner as i` at the file level and `use super::twice` inside a
        nested inline module (an earlier note claimed `super::` panicked the
        module loader; that no longer reproduces, and the nested-module form is
        now part of the corpus).
      The AOT differential then measured the new corpus and recorded these gaps
      with `@no-aot` (each one must keep failing, so closing it forces a diff):
      `defer`, match guard/or/range/type patterns, `spawn`/`lock`/service,
      nested tuple access and tuple pass/return, trait dispatch through a
      generic parameter, and failure-path reporting (the interpreter buffers
      `print` and writes `error[E0006]`, the native runtime writes `panic: …`;
      see T13). Block-scoped shadowing, `f32` arithmetic, and the `Error("…")`
      constructor were closed later: the AOT path now restores bindings at every
      nested block, gives `f32`/`f64` bindings an F64 slot, and builds the
      `{ message }` object `Error("…")` denotes, so all three agree with the
      interpreter and their markers are gone.
- [x] **T9** Measure stage 1 against the same corpus.
      `xuloc/main.xulo` gained a `scan` mode; `XULO_STAGE1_MODE=scan
      XULO_STAGE1_DIR=examples xulo run xuloc/main.xulo` prints one line per file
      with the lexer and parser verdict. `crates/xulo-cli/tests/stage1.rs::
      stage1_example_corpus_tracks_baseline` compares that with
      `crates/xulo-cli/tests/stage1_examples_baseline.tsv` and fails when a listed
      gap starts passing. Current baseline: 62 files, 12 parser gaps and 1
      call-depth crash (`std/net.xulo` nests four `match` levels), 0 lexer gaps.

**Verify:** `cargo test -p xulo-cli --test examples`,
`cargo test -p xulo-cli --test stage1 stage1_example_corpus_tracks_baseline`.

## WP4 — `defer` semantics

Priority P1 · needed by Gate 3 · source: `stage1-readiness.md` §7 WP4, §6.8

- [x] **L20a** Add a per-block defer queue in the interpreter that registers
      without evaluating the operand.
- [x] **L20b** Run deferred statements at scope exit in reverse registration
      order.
- [x] **R7** Run deferred statements before `return`, `break`, and `?`
      propagation.
- [x] **B3b** Decide whether the AOT path supports `defer`; if not, document it
      as a bootstrap restriction instead of silently executing it immediately.
- [x] **G4** Make the `§6.8` probe pass.

**Verify:** `defer print("first")` / `defer print("second")` / `print("body")`
prints `body`, `second`, `first`. Covered by
`crates/xulo-runtime/tests/defer.rs`; the AOT backend rejects `defer` with a
named bootstrap-restriction diagnostic (no silent skip), tested in
`crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs`.

## WP5 — Stage 1 skeleton

Can start in parallel with WP1–WP4 · needed by Gate 1/2 · source:
`stage1-readiness.md` §7 WP5

- [x] Create the agreed `xuloc/` location and a lexer written in Xulo.
      `xuloc/ast.xulo` and `xuloc/lexer.xulo`; run with
      `XULO_STAGE1_MODE=tokens XULO_STAGE1_FILE=<file> xulo run xuloc/main.xulo`.
- [x] Add a parser written in Xulo that runs under `xulo run`.
      `xuloc/parser.xulo` and `xuloc/main.xulo`; `parse` mode prints the
      generic AST tree with scalar spans.
- [x] Limit the source to rows marked OK/PARTIAL and record every workaround
      in `xuloc/BOOTSTRAP.md`.
      The document lists the sixteen deliberate limits, including the
      `xulo run` argv workaround, cross-module method dispatch, and the
      unsupported postfix `?`, call-site type arguments, and arrow closures.
- [x] **T5a** Smoke test: stage 1 parses a fixed example and prints
      spans/tokens.
      `crates/xulo-cli/tests/stage1.rs` runs `tokens` and `parse` on
      `examples/functions/fibonacci.xulo` and asserts spans and node positions.
- [x] **T5b** Smoke test: stage 1 parses its own source.
      The same test parses every `xuloc/*.xulo`; `xuloc/tests.xulo` repeats
      the check through its own `#[test]` functions.
- [x] **B6** Verify `xupm test` runs stage 1's own `#[test]` functions.
      `crates/xupm/tests/cli.rs::runs_stage1_test_file` and
      `crates/xulo-cli/tests/stage1.rs::stage1_tests_run_under_xulo_test`.

**Verify:** `cargo test -p xulo-cli --test stage1` and
`cargo test -p xupm --test cli runs_stage1_test_file`.

**Exit:** Gate 1 and Gate 2 — reached on 2026-11-16.

## WP6 — Self-host loop

After WP1–WP5 · needed by Gate 4/5/6 · source: `stage1-readiness.md` §7 WP6

- [x] Stage 1 checks and compiles a fixed bootstrap corpus.
      `xuloc/backend.xulo` lowers the generic AST to portable C and links
      `xuloc/runtime.c`; the accept corpus compiles and prints its checked-in
      goldens, and the reject corpus still fails with a coded diagnostic
      (`crates/xupm/tests/stage1_build.rs::stage1_compiles_the_bootstrap_corpus`).
- [x] **B8a** Build stage 1 with stage 0's `xupm`.
      `xuloc/` is an `xupm` package (`xuloc/xupm.toml`, entry `main.xulo`), so
      `xupm build xuloc/main.xulo --release` produces
      `xuloc/.xupm/build/xuloc`; the native binary matches the interpreter for
      `tokens`/`parse` on `examples/functions/fibonacci.xulo` and `parse` on
      `xuloc/main.xulo`. The opt-in differential test rejects output drift.
      `crates/xupm/tests/cli.rs::xuloc_is_a_package_project` keeps the manifest
      and the no-file-argument check path working.
- [x] Stage 1 builds itself, producing a stage 2 binary.
      `crates/xupm/tests/stage1_build.rs::stage1_builds_itself_into_stage2`.
- [x] **T5c** Compare stage 1 and stage 2 output on the bootstrap suite.
      `::stage1_and_stage2_agree_on_the_bootstrap_suite` compares tokens and
      parse trees for every `xuloc/*.xulo`, check verdicts/diagnostics for the
      accept and reject corpus, and the runtime output of every corpus build.
- [x] Add the bootstrap suite to CI alongside the conformance runner.
      The `conformance` job runs `cargo test -p xulo-cli --test stage1`,
      `cargo test -p xupm --test cli runs_stage1_test_file`, and the opt-in
      `XULO_STAGE1_BUILD=1 cargo test -p xupm --test stage1_build`, so the
      corpus, `xuloc/tests.xulo`, and the self-host loop gate alongside the
      pinned-spec runner.
- [x] **B8b** Add a reproducible build and self-compile smoke test.
      `::stage1_build_is_reproducible` builds `basics.xulo` twice and compares
      the generated C byte for byte, then builds the compiler with stage 2 and
      requires the stage 2 translation unit to match stage 1's exactly.
- [x] **D4** Make stage 1 diagnostics consistent with stage 0 for the
      bootstrap subset.
      Lexical/syntax diagnostics use `E0001`/`E0002`; the checker reports
      `E0101`/`E0201`/`E0202`/`E0207`/`E0211`/`E0213`/`E0215` with
      `file:line:column`. Stage 0 now reports the specific code for undefined
      names, conditions, arity, `for` iterables, and return mismatches, and
      `crates/xulo-cli/tests/stage1.rs::stage1_check_agrees_with_stage0_on_the_corpus`
      compares the codes, not only accept/reject.

**Verify:** `cargo test -p xulo-cli --test stage1` plus
`XULO_AOT_DIFFERENTIAL=1 XULO_RUNTIME_LIB=$PWD/target/release/libxulo_runtime.a
cargo test -p xupm --test aot_differential stage1_binary_matches_the_interpreter`,
and the WP6 self-host runner:

```bash
XULO_STAGE1_BUILD=1 cargo test -p xupm --test stage1_build -- --nocapture
```

It needs a C compiler (set `XULO_CC` when the host `clang` is not on `PATH`)
and builds the matching `xulo-runtime` static library itself. The self-build is
CPU-heavy: the full runner takes roughly four minutes on Windows.

**Exit:** Gate 4, Gate 5, Gate 6 — all reached (2026-11-18).

## WP7 — Non-gating follow-ups

Not in any gate's exit criterion. Each item must still be closed or explicitly
re-scoped before the gate it informs.

- [x] **T3f** Triage the 33 divergences in `spec_conformance_baseline.tsv` by
      group: 11 context-dependent, 12 module-scope statements, 8 syntax gaps,
      2 semantic gaps. Each group names an owner component and the gate that
      clears it in the table in `stage1-readiness.md` §6.10 (module-scope and
      context-dependent at Gate 6; syntax and semantic at Gate 5); the component
      paths are checked by `readiness_matrix`. "Deadline" for these groups means
      the gate named in "Cleared by"; there is no separate calendar date.
- [x] **T3c** Extend the runner to `// runs:` blocks with expected output.
      The pinned spec has no `runs:` examples, so local cases live under
      `crates/xulo-cli/tests/runs_cases/basics.md`; the runner scans them
      alongside the pinned spec and fails if the executable path loses all
      cases.
- [x] **L21** `copy` / `move` and use-after-move checking.
      Both operators are accepted only in a `let` initializer, a call
      argument, or a `return` (E0403), `move` requires a whole frame-owned
      binding, and any later use of a moved binding is E0403.
      `crates/xulo-semantic/tests/semantic_tests.rs` covers the position,
      borrow, twice-move, and use-after-move cases.
- [x] **R8** Value/reference semantics for `copy` / `move`. `copy` deep-copies
      lists, maps, sets, builders, structs, enums, and tuples in both paths and
      preserves identity-carrying values; `move` transfers the value and
      invalidates the binding. Plain binding, assignment, destructuring, loop,
      and match binding now apply the spec's value/reference table: `struct`,
      `enum`, `Array`, tuple, and `Range` values get their own container, while
      `List`, `Map`, `Set`, and `StringBuilder` keep sharing. Evidence:
      `crates/xulo-runtime/tests/value_semantics.rs` and
      `crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs::bindings_copy_value_aggregates_and_share_references`.

- [x] **P1 — Aggregate `const` initializers are re-evaluated at every reference
      in AOT.** Aggregate constants now get a stable slot: the entry function
      materializes the initializer once before its body, every other reference
      reads the slot through `RuntimeFn::ConstGet`, and the runtime keeps the
      values in a thread-local slot table (`xulo_const_reset/get/set`) that is
      cleared before each entry run. Scalar constants keep inlining. This
      removes the per-reference array allocation and element re-interning that
      made `xuloc`'s `is_keyword` scans allocate a fresh `KEYWORDS` list for
      every identifier token. On the `tokens` probe (22 KB of `let x = 1`
      lines) the AOT stage 1 binary drops from 101.9 s to 83.3 s. Evidence:
      `crates/xulo-compiler/tests/const_materialization_tests.rs`
      (`aggregate_const_initializer_runs_once_before_main`,
      `aggregate_const_references_share_one_value`).

- [x] **P2 — AOT tag auto-detection scans growing registries linearly.**
      `xulo_detect_tag` walked the string, array, object, enum, regex, and
      float registration vectors on every call; with per-token `str()` calls
      this made stage 1's `tokens` mode quadratic even after P1. Each registry
      now keeps an O(1) pointer/value membership set next to its vector,
      maintained at the single registration/allocation site and cleared by
      `xulo_cleanup`. Combined with P1 the `tokens` probe scales linearly:
      14 KB 0.06 s, 22 KB 0.10 s, 44 KB 0.20 s (previously 339 s at 44 KB),
      and its output is byte-identical to the old stage 1 binary on
      `xuloc/lexer.xulo`. Evidence:
      `crates/xulo-runtime/tests/tag_detection.rs`.

- [x] **T10** Cover the `std` surface in `examples/std/`, and measure it.
      The corpus gate proves grammar coverage only: `examples/coverage.tsv` maps
      every production of `spec/grammar.md` to an example, but nothing compared
      the corpus with `std-spec/signature-index.md` (146 member rows, 143
      distinct names — `format`, `parse`, and `read` each appear in two
      modules). Comparing that index with the expressions actually written in
      `examples/` left 38 callable members uncalled; they are now covered by
      `examples/std/math_extra.xulo` (23 `std::math`),
      `examples/std/fs_extra.xulo` (8 `std::fs`),
      `examples/std/io_extra.xulo` (5 `std::io`),
      `examples/std/net_shutdown.xulo` (`sock_shutdown`), and
      `examples/std/regex.xulo` (the string member `split`;
      `std::path::split` is a different member). `process::Child`,
      `http::Server`, and `regex::RegexMatch` are never spelled out but are used
      through inference, so they are not gaps. All 38 also have an AOT lowering
      (`crates/xulo-compiler/src/irgen/mod.rs::intrinsic_lowering`, plus the
      `split` method path) and produce output identical to the interpreter, so
      the new examples need no `@no-aot` marker.
      `crates/xulo-cli/tests/examples.rs::std_members_are_exercised_by_the_corpus`
      then compares the index with the corpus by `std::<module>::<member>`
      (`.split(` for the `string.*` pseudo members), ignores comments so a
      member named only in prose does not count, skips `type …` rows, and skips
      when the `std-spec` submodule is absent. Non-gating: no gate's exit
      criterion names the standard-library surface. Writing the examples found
      two AOT lowering bugs, both fixed with regressions: `+` on a payload-bound
      string/list went through the dynamic-number path, and `string.split` with
      a compiled `RegExp` was lowered as a separator string.

**Verify:** `cargo test -p xulo-cli --test readiness_matrix` (T3f owner
components), `cargo test -p xulo-cli --test spec_conformance -- --nocapture`
(T3c), `cargo test -p xulo-semantic --test semantic_tests` (L21), the R8
value-semantics tests listed above, `cargo test -p xulo-compiler --test
const_materialization_tests` (P1), and `cargo test -p xulo-runtime --test
tag_detection` (P2).

- [x] **T11** AOT: namespace user function symbols.
      The native backend emitted each user `fn` under its source name, so a
      function called `listen`, `connect`, `accept`, `read`, `write`, `open`, or
      `close` collided with the C library / Winsock import of the same name and
      the link failed (`LNK2005: listen already defined`). Found while writing
      `examples/std/net_shutdown.xulo`, whose handshake helper had to be named
      `open_listener`. User functions now emit as `xulo_fn_<name>`
      (`crates/xulo-compiler/src/codegen.rs::aot_function_symbol`); the entry
      point stays `main` (the C runtime calls it) and `pub extern "C" fn`
      exports keep the declared name (a host links against it), which
      `IrFunction::exported` records. Both backends use the helper, and
      `crates/xulo-compiler/tests/aot_symbols.rs` pins the namespacing, the
      `main` entry, and the unmangled extern export.

- [ ] **T12** Enforce the `f32` format in both engines.
      `spec/types/primitive-types.md` requires IEEE-754 semantics "for their
      format", so an `f32` literal must round to binary32 and every `f32`
      arithmetic result must land back in binary32 before it is printed or
      promoted. Today both the interpreter and the AOT path compute in `f64`:
      `let o: f32 = 0.8` prints `0.8` instead of `0.800000011920929`, and
      `1.0f32 / 3.0` keeps all the double's digits. `types/float_math.xulo`
      stays on values both formats represent exactly, so it does not gate this.
      Fix the coercion at the binding site and the rounding after each
      operation, then add a differential example with a value that differs
      between the two formats.

- [ ] **T13** Decide the failure-path reporting contract.
      Two examples stay `@no-aot` because a runtime failure prints different
      things on the two paths, and neither is a bug in the lowering:
      the interpreter's CLI renders `error[E0006] runtime: …` with a source
      snippet (` --> path:line:col`) and drops the buffered stdout, while the
      native runtime streams stdout as it runs and prints `panic: <message>`.
      Byte parity therefore needs one of: (a) the compiler embedding the
      rendered diagnostic (message, canonical path, source line, caret) in the
      binary and having the runtime print it before exiting, (b) a
      runtime-only message contract that the interpreter also adopts for
      failures (losing the snippet), or (c) keeping the two formats and
      teaching the differential to compare a normalized failure shape for
      examples that declare it. Pick one before `errors/runtime_failure.xulo`
      and `errors/division_by_zero.xulo` can leave the gap list.

## Out of scope for stage 1 bootstrap

These are deliberately not tracked here. They remain P2 in
`stage1-readiness.md` §8 and must not block Gate 1 or Gate 2:

- UI libraries and renderers (the language defines no UI layer; UI is a library
  concern).
- LSP, IDE integration, and formatter comment handling.
- Async tasks, `spawn` / `lock` / `shared`, `std::net`, `std::http`, package
  registry.
- Macro expansion beyond `#[test]` in stage 1's own tests.
- **S8** Full FFI and Windows FFI.

## Maintenance

- Update this file whenever a task changes state; keep `stage1-readiness.md`
  in sync in the same change.
- Semantic drift between the execution engines, the stage-comparison invariants
  (I1–I4), and the deviation ledger are governed by
  [`drift-governance.md`](drift-governance.md). Update its ledger in the same
  change that fixes or introduces a divergence.
- At each gate review, record the date, the stage 1 commit, the stage 0 commit,
  and the pinned spec commits.
- If the bootstrap subset changes, update the affected tasks and the matching
  rows in `stage1-readiness.md` §2.1.
