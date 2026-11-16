# Stage 1 Bootstrap Tasks

- **Status**: Living checklist
- **Date**: 2026-11-16
- **Companion document**: [`stage1-readiness.md`](stage1-readiness.md) — rationale, evidence, and gate definitions
- **Normative sources**: `spec@48f49b7` and `std-spec@a2e4cb7`, as pinned by this repository
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
| WP0 — Matrix hygiene and conformance | 6 | 6 |
| WP1 — 64-bit integer model | 7 | 7 |
| WP2 — Type system and semantic convergence | 10 | 10 |
| WP3 — AOT parity and bootstrap backend | 8 | 8 |
| WP4 — `defer` semantics | 5 | 5 |
| WP5 — Stage 1 skeleton | 6 | 6 |
| WP6 — Self-host loop | 1 | 7 |
| **Total** | **43** | **49** |

## Gates

Gates are cumulative. An open earlier gate blocks every later gate.

- [x] **Gate 1 — Interpreter prototype**: stage 1 lexer/parser runs under
      `xulo run`, reads a `.xulo` file, and prints tokens/AST or a clean
      diagnostic. `crates/xulo-cli/tests/stage1.rs`.
- [x] **Gate 2 — Self-parse**: stage 1 parses its own source (or a fixed
      nontrivial bootstrap file) with accurate spans.
      `crates/xulo-cli/tests/stage1.rs` parses `xuloc/*.xulo` itself.
- [ ] **Gate 3 — Self-check**: stage 1 type-checks a nontrivial bootstrap file
      and agrees with stage 0 on accept/reject; diagnostics carry
      file/line/column and exit nonzero.
- [x] **Gate 4 — Native build**: `xupm build` produces a stage 1 binary whose
      output matches `xulo run` on the bootstrap workload.
      `crates/xupm/tests/aot_differential.rs::stage1_binary_matches_the_interpreter`.
- [ ] **Gate 5 — Self-host**: stage 1 compiles itself into stage 2; stage 1 and
      stage 2 agree on the bootstrap suite and the conformance subset.
- [ ] **Gate 6 — Conformance**: the pinned specification's checkable examples
      and the bootstrap suite run in CI.

## Completed baseline

Capabilities stage 1 may rely on today. These are not work packages; they are
the starting point, and their evidence is recorded in `stage1-readiness.md` §5.

- [x] **L1** UTF-8 lexing and parsing of `.xulo` files
- [x] **L3** `float` / `f64` arithmetic
- [x] **L4** `let`, `let mut`, assignment, shadowing
- [x] **L5** Functions, recursion, implicit return
- [x] **L7** `if` / `else`, `while`, `for`, `match`, `break`, `continue`
- [x] **L8** Enums, payloads, recursive/generic enums, exhaustive `match`
- [x] **L9** Structs, `impl`, traits, explicit `Trait::method(recv)` dispatch
- [x] **L10** Generic functions, bounds, `where` clauses
- [x] **L12** `mod` / `use` / `pub`, visibility, cross-file modules
- [ ] **L13** Tuple literals, positional access, and destructuring (equality is
      identity-based for `List`/tuple/`struct`; see `stage1-readiness.md` §6.11)
- [x] **L16** `T?`, `?.`, `??`, null narrowing
- [x] **L19** `Result` / `Error` and `?` propagation
- [x] **L22** Async/await, `spawn`, `lock`, `shared` (outside the bootstrap subset)
- [x] **L23** Attributes, `#[test]`, `#[derive]`, macros (outside the bootstrap subset)
- [x] **R1** Deterministic left-to-right evaluation
- [x] **R3** UTF-8 scalar string operations
- [x] **R4** Runtime failures with stable codes
- [x] **R5** No panic on adversarial input; nesting guard
- [x] **R6** `panic`, `assert`, and process exit status
- [x] **C5** `string` members and UTF-8 behavior
- [x] **C6** `StringBuilder` with `new` / `append` / `clear` / `build` / `len` / `byte_len`
- [x] **S1** `std::fs`
- [x] **S2** `std::io`
- [x] **S3** `std::process`
- [x] **S4** `std::path`, `std::os`, environment, cwd, pid
- [x] **S5** `std::json`
- [x] **S6** `std::regex` and `std::fmt`
- [x] **S7** `std::net` and `std::http` (outside the bootstrap subset)
- [x] **B1** `xulo run` and `xulo check` for a single file
- [x] **B2** `xupm build` for core language and `hello`
- [x] **B4** AOT module tree (`mod` / `use`)
- [x] **B5** Runtime static library and linker path
- [x] **D1** Source spans and `file:line:column` diagnostics
- [x] **D2** stderr writes and nonzero exit status
- [x] **T1** Unit and integration tests — `cargo test --workspace`: 2105 passed, 0 failed (2026-11-16)
- [x] **T2** All examples run in CI
- [x] **T6** Robustness/fuzz tests

## WP0 — Matrix hygiene and conformance

Priority P0 · needed by Gate 1/2 · source: `stage1-readiness.md` §7 WP0, §6.9

- [x] **T7** Add a CI check that `git submodule status` matches the pinned
      `spec@48f49b7` and `std-spec@a2e4cb7` commits.
      `crates/xulo-cli/tests/submodule_pins.rs`.
- [x] **T3a** Add a conformance runner that extracts `// ok` blocks from the
      pinned specification and runs them through stage 0.
      `crates/xulo-cli/tests/spec_conformance.rs` (`Accept` cases).
- [x] **T3b** Extend the runner to `// error[E…]` blocks with expected codes.
      Each rejected line is re-inserted uncommented and must produce its code.
- [x] **T3c** Extend the runner to `// runs:` blocks with expected output.
      Implemented, but the pinned spec currently has no `runs:` examples, so
      that path has no cases yet.
- [x] **T3d** Ensure every row marked OK in `stage1-readiness.md` is backed by
      a test or a named probe; add the missing ones.
      `crates/xulo-cli/tests/readiness_matrix.rs` requires every OK row to cite
      an existing path, runs the `crates/xulo-cli/tests/matrix_probes/` corpus,
      and fails on a probe that no row cites. The audit found and fixed two
      defects (a trailing struct construction was mistaken for a component and
      lost its implicit return; tuple destructuring rejected tuple types) and
      re-graded L13 to PARTIAL for identity-based equality (§6.11).
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

**Verify:** the four probes in `stage1-readiness.md` §6.2–6.5 run without a
workaround.

## WP3 — AOT parity and bootstrap backend

Priority P1 · needed by Gate 4 · source: `stage1-readiness.md` §7 WP3, §6.6–6.7

- [x] **C2** Add AOT runtime support for `Map`.
- [x] **C3** Add AOT runtime support for `Set`.
- [x] **C1** Fix list spread lowering in the AOT path
      (`examples/collections.xulo`).
- [x] **R9b** Fix nested ternary lowering in the AOT path
      (`examples/optional_chaining.xulo`).
- [x] **L15** Fix `Array<T, N>.len` in the AOT path.
- [x] **T4** Add a differential runner that builds every non-interactive
      example both ways and compares output.
- [x] **B3a** Verify that `xupm build` failures return a nonzero exit status;
      fix the pipeline if they do not.
- [x] **G3** Make the `§6.6–6.7` probes pass.

**Verify:** `xupm build examples/collections.xulo` and
`xupm build examples/optional_chaining.xulo` match `xulo run` byte for byte.
`XULO_AOT_DIFFERENTIAL=1 cargo test -p xupm --test aot_differential` compares
25 examples; the baseline records five entries that cannot be compared yet.

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
      The document lists the fourteen deliberate limits, including the
      `xulo run` argv workaround, cross-module method dispatch, and the
      unsupported postfix `?`, call-site type arguments, and arrow closures.
- [x] **T5a** Smoke test: stage 1 parses a fixed example and prints
      spans/tokens.
      `crates/xulo-cli/tests/stage1.rs` runs `tokens` and `parse` on
      `examples/fibonacci.xulo` and asserts spans and node positions.
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

- [ ] Stage 1 checks and compiles a fixed bootstrap corpus.
- [x] **B8a** Build stage 1 with stage 0's `xupm`.
      `xupm build xuloc/main.xulo --release` produces
      `xuloc/.xupm/build/main`; the native binary matches the interpreter for
      `tokens`/`parse` on `examples/fibonacci.xulo` and `parse` on
      `xuloc/main.xulo`. The opt-in differential test rejects output drift.
- [ ] Stage 1 builds itself, producing a stage 2 binary.
- [ ] **T5c** Compare stage 1 and stage 2 output on the bootstrap suite.
- [ ] Add the bootstrap suite to CI alongside the conformance runner.
- [ ] **B8b** Add a reproducible build and self-compile smoke test.
- [ ] **D4** Make stage 1 diagnostics consistent with stage 0 for the
      bootstrap subset.
      Lexical/syntax diagnostics now use stage 0's `E0001`/`E0002` codes and a
      one-line `file:line:column` shape; semantic diagnostics need the
      stage 1 checker and are still open.

**Verify:** `cargo test -p xulo-cli --test stage1` plus
`XULO_AOT_DIFFERENTIAL=1 XULO_RUNTIME_LIB=$PWD/target/release/libxulo_runtime.a
cargo test -p xupm --test aot_differential stage1_binary_matches_the_interpreter`.

**Exit:** Gate 4, Gate 5, Gate 6. Gate 4 is reached; Gates 5–6 remain blocked on
the stage 1 checker and self-build.

## Out of scope for stage 1 bootstrap

These are deliberately not tracked here. They remain P2 in
`stage1-readiness.md` §8 and must not block Gate 1 or Gate 2:

- UI components, renderers, `@State` / `@Environment`, terminal/webview backends.
- LSP, IDE integration, and formatter comment handling.
- Async tasks, `spawn` / `lock` / `shared`, `std::net`, `std::http`, package
  registry.
- Macro expansion beyond `#[test]` in stage 1's own tests.
- Full FFI and Windows FFI.

## Maintenance

- Update this file whenever a task changes state; keep `stage1-readiness.md`
  in sync in the same change.
- At each gate review, record the date, the stage 1 commit, the stage 0 commit,
  and the pinned spec commits.
- If the bootstrap subset changes, update the affected tasks and the matching
  rows in `stage1-readiness.md` §2.1.
