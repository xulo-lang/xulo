# Stage 1 Bootstrap Readiness

- **Status**: Living document
- **Date**: 2026-11-17
- **Audience**: compiler/runtime maintainers, stage 1 bootstrap team
- **Normative sources**: `spec@48f49b7` and `std-spec@a2e4cb7`, the commits pinned by this repository
- **Scope**: the capabilities stage 0 must provide for stage 1, their current status, and the gates for calling stage 0 "host-ready"

This is a bootstrap-readiness document, not a language specification. The
language specification remains normative for what Xulo programs mean; this
document only records what stage 1 needs from stage 0, what exists today, and
what is still missing. The executable checklist derived from this document is
[`tasks.md`](tasks.md); keep the two in sync.

> **Repository state note.** The superproject pins `spec@48f49b7` and
> `std-spec@a2e4cb7`, but a working tree can be checked out at a different
> submodule commit. Run `git submodule status` before using this matrix.
> Evaluating against a stale specification checkout produces false mismatches
> (for example `stringSize` vs. string members, or `#Test` vs. `#[test]`).
> This document is written against the pinned commits above.

## 1. Purpose

Stage 0 is the Rust bootstrap toolchain in this repository. Stage 1 is a
compiler or toolchain written in Xulo itself, checked/run/built by stage 0.
Stage 2 is stage 1 compiled by stage 1, used to verify self-hosting.

This document answers three questions:

1. What must stage 0 provide before stage 1 development can start?
2. Which requirements are met today, and what is the evidence?
3. What must be fixed, in what order, before each bootstrap gate?

It is deliberately a checklist with measurable gates, not a design essay. A
row marked "OK" must have a test or a named verification command; a row without
one is "NOT VERIFIED", not "OK".

## 2. Definitions and assumptions

- **Stage 0**: the Rust `xulo` CLI, interpreter, standard library, and `xupm`
  AOT toolchain in this repository.
- **Stage 1**: a Xulo-written implementation of the bootstrap subset (initially
  a lexer/parser/checker, later a working compiler) that stage 0 can check and
  run.
- **Stage 2**: stage 1 compiled by stage 1. A successful stage 2 is the first
  real self-hosting signal.
- **Bootstrap subset**: the part of the pinned language and standard library
  that stage 1 source is allowed to use. The default target is the full pinned
  specification; any narrowing must be written down in this section and kept
  deliberately small.

### 2.1 Bootstrap subset for stage 1 source

The following is the intended starting subset. It is chosen for a compiler
front end, not for UI or network applications:

- Primitives: `bool`, `int`, `float`, `string`, `unit`, `unknown`.
- Functions, recursion, closures, `let`/`let mut`, assignment, `if`/`else`,
  `while`, `for`, `match`, `break`, `continue`, `return`.
- `struct`, `enum` (payload and recursive), `impl`, `trait`, generics and
  bounds, type aliases.
- `List`, `Map`, `Set`, `Array`, `StringBuilder`, tuples.
- Optional types, `?.`, `??`, null narrowing, `is`/`is not` narrowing.
- `Result`/`Error` with `?` propagation.
- `defer` (needed for temporary files and cleanup in a compiler).
- `mod`/`use`/`pub` and module visibility.
- Standard library: `std::fs`, `std::io`, `std::process`, `std::path`,
  `std::os`, `std::math`, `std::time`, `std::fmt`, `std::json`,
  `std::regex` if needed.
- `#[test]` and `assert` for stage 1's own tests.

Explicitly **not required** for the bootstrap subset: UI/components,
`@State`/`@Environment`, renderers, LSP, formatter comments, async tasks,
`spawn`/`lock`/`shared`, macros, `std::net`, `std::http`, package registry,
and non-Unix FFI. These may remain P2 and must not block Gate 1 or Gate 2.

### 2.2 Execution-path assumption

Stage 1 starts on the interpreter:

```text
stage1.xulo --(xulo run, stage 0 interpreter)--> running stage 1
```

The native path becomes mandatory at Gate 4:

```text
stage1.xulo --(xupm build, stage 0 AOT)--> stage1 binary
```

This distinction matters: a missing AOT feature does not block the first
prototype, but it does block a native bootstrap.

## 3. Status legend

| Status | Meaning |
|--------|---------|
| **OK** | Implemented and verified by a test or a named probe. |
| **PARTIAL** | Usable with a documented workaround or only on one execution path. |
| **MISSING** | The capability does not exist. |
| **BROKEN** | The capability exists but produces the wrong observable behavior. |
| **NOT VERIFIED** | Believed to exist, but no test or probe currently proves it. |

## 4. Bootstrap gates

Each gate has an exit criterion. Gates are cumulative: do not claim a later
gate while an earlier one is open.

### Gate 1 — Interpreter prototype

**Exit criterion:** stage 1's lexer and parser run under `xulo run`, read a
`.xulo` file, and print tokens/AST or a clean diagnostic. The source uses only
rows marked OK/PARTIAL in this document, and every workaround is recorded in
`xuloc/BOOTSTRAP.md`.

**Current assessment:** **Reached (2026-11-16).** `xuloc/main.xulo` runs the
`xuloc/lexer.xulo` and `xuloc/parser.xulo` front end under `xulo run`, reads a
file through `XULO_STAGE1_FILE`, and prints either tokens or the AST tree (or a
clean `file:line:column` diagnostic and exit 1). Evidence:
`crates/xulo-cli/tests/stage1.rs`. Every workaround is recorded in
`xuloc/BOOTSTRAP.md`.

### Gate 2 — Self-parse

**Exit criterion:** stage 1 parses its own source (or a fixed, nontrivial
bootstrap file) and reports accurate spans. Round-trip or structural checks
cover literals, expressions, declarations, and modules.

**Current assessment:** **Reached (2026-11-16).** Stage 1 parses all six of its
own source files (`ast`, `lexer`, `parser`, `checker`, `main`, `tests`) and
reports scalar spans and 1-based line/column. The test discovers `xuloc/*.xulo`
instead of listing them, so a new stage 1 source file cannot silently escape
self-parsing. A generic `Node` plus `unknown`-free scalar
fields replaced the earlier tuple/`unknown`/`is` design risk; the trade-offs are
listed in `xuloc/BOOTSTRAP.md`. Evidence:
`crates/xulo-cli/tests/stage1.rs::stage1_parses_its_own_source` and
`xuloc/tests.xulo::parses_its_own_source`.

### Gate 3 — Self-check

**Exit criterion:** stage 1 type-checks a nontrivial bootstrap file and agrees
with stage 0 on accept/reject for the bootstrap subset. Diagnostics include
file/line/column and exit nonzero on failure.

**Current assessment:** **Reached (2026-11-16).** `xuloc/checker.xulo` checks
the fixed corpus under `xuloc/corpus/` (functions, bindings, control flow,
lists, strings, calls, return types). Stage 1 and stage 0 agree on accept/reject
for both accepted files and all six rejected files, and D4 aligns their
diagnostic codes as well; rejected diagnostics carry `file:line:column`, an `E…`
code, and exit 1. Evidence:
`crates/xulo-cli/tests/stage1.rs::stage1_check_agrees_with_stage0_on_the_corpus`
and `xuloc/tests.xulo::checker_*`.

### Gate 4 — Native build

**Exit criterion:** `xupm build` produces a stage 1 binary that runs the same
bootstrap workload as `xulo run` and prints identical output.

**Current assessment:** **Reached (2026-11-16).** `xupm build
xuloc/main.xulo --release` produces a native stage 1 binary, and its stdout and
stderr match `xulo run` for `tokens`/`parse` on `examples/fibonacci.xulo` and
`parse` on `xuloc/main.xulo`. Evidence:
`crates/xupm/tests/aot_differential.rs::stage1_binary_matches_the_interpreter`,
run by the AOT differential CI job, which sets `XULO_AOT_DIFFERENTIAL=1` and
`XULO_RUNTIME_LIB`; a plain `cargo test --workspace` does not run it. The
assessment covers Linux and macOS: macOS was verified locally on 2026-11-17
(all 30 examples and the stage 1 workload match byte for byte), and CI now runs
the differential on both hosts. Windows runs the same
differential with a platform baseline that parks `ffi.xulo` (Unix-only FFI,
S8); its first CI run is pending, so cross-platform coverage remains open as B7
in `tasks.md` WP3.

### Gate 5 — Self-host

**Exit criterion:** stage 1 compiles itself, producing a stage 2 binary; stage 1
and stage 2 agree on the bootstrap suite and on the conformance subset. A
failure in either direction is a release blocker.

**Current assessment:** **Blocked on stage 1's checker and code generator.**
The 64-bit integer model and the AOT stage 1 build are in place (WP1, B8a), so
the remaining prerequisites are the stage 1 semantic pass and a stage 1 backend
that can emit a stage 2 binary. Gate 4 already proves the bootstrap workload
survives the native path.

### Gate 6 — Conformance

**Exit criterion:** the pinned specification's checkable examples and the
bootstrap suite run in CI; stage 1 and stage 0 agree on the bootstrap subset;
known deviations are listed with the component that must change and the gate
that closes them.

**Current assessment:** **Runner in place, bootstrap suite open.** The pinned
specification's checkable examples run in the `conformance` job
(`crates/xulo-cli/tests/spec_conformance.rs`), with known divergences recorded
in `spec_conformance_baseline.tsv`. The checker corpus is now part of
`cargo test --workspace`; the full bootstrap suite still waits on stage 1 code
generation and the self-hosted build (Gates 5–6). Owners and the gate that
clears each §6.10 group are recorded in §6.10 (`tasks.md` T3f).

## 5. Capability matrix

### 5.1 Language and type system

Priority is relative to Gate 1/2 unless stated otherwise.

| ID | Requirement | Status | Evidence / limitation | Needed by |
|----|-------------|--------|-----------------------|-----------|
| L1 | UTF-8 lexing and parsing of `.xulo` files | **OK** | Unicode comments and literals survive lexing/parsing and reach the runtime; `crates/xulo-cli/tests/matrix_probes/L1_utf8_lexing.xulo`, `crates/xulo-parser/tests/tokenize_tests.rs` | 1 |
| L2 | `int` is a 64-bit signed integer; `u64` covers `0…2^64-1` | **OK** | `Literal::Int(i128)` + `Literal::Float(f64)`; `Value::Int(i64)`/`UInt(u64)`/`Float(f64)`; integers are exact above `2^53` and through `u64::MAX`; a literal above the `u64` range is `E0209`; `crates/xulo-runtime/tests/integers.rs` | 1 |
| L3 | `float`/`f64` arithmetic | **OK** | `f64` literals, arithmetic, and printing; `crates/xulo-cli/tests/matrix_probes/L3_float_arithmetic.xulo` | 1 |
| L4 | `let`, `let mut`, assignment, shadowing | **OK** | Bindings, assignment, and nested-block shadowing; `crates/xulo-cli/tests/matrix_probes/L4_bindings.xulo`, `crates/xulo-semantic/tests/semantic_tests.rs` | 1 |
| L5 | Functions, recursion, implicit return | **OK** | Recursion, implicit return, and default parameters; `crates/xulo-cli/tests/matrix_probes/L5_functions.xulo` | 1 |
| L6 | Closures and function values | **OK** | An untyped lambda passed to a generic callback infers the type parameter from the other arguments (`untyped_closure_infers_generic_callback_type`, `generic_closure_argument_compiles`); `crates/xulo-semantic/tests/semantic_tests.rs`, `crates/xulo-compiler/tests/type_system_codegen_tests.rs` | 2 |
| L7 | `if`/`else`, `while`, `for`, `match`, `break`, `continue` | **OK** | `crates/xulo-cli/tests/matrix_probes/L7_control_flow.xulo` | 1 |
| L8 | Enums, payloads, recursive/generic enums, exhaustive `match` | **OK** | Payload and recursive generic enums; `crates/xulo-cli/tests/matrix_probes/L8_enums_match.xulo`, `examples/enums_match.xulo` | 1 |
| L9 | Structs, `impl`, traits, explicit `Trait::method(recv)` dispatch | **OK** | `crates/xulo-cli/tests/matrix_probes/L9_structs_traits.xulo`, `examples/trait_shapes.xulo` | 1 |
| L10 | Generic functions, bounds, `where` clauses | **OK** | Inline and `where` bounds on a generic function; `crates/xulo-cli/tests/matrix_probes/L10_generics.xulo`, `examples/generics.xulo` | 2 |
| L11 | Generic struct construction with named arguments | **OK** | The constructor infers type arguments from named field arguments and member access substitutes them; `crates/xulo-runtime/tests/type_system.rs`, `crates/xulo-compiler/tests/type_system_codegen_tests.rs` | 2 |
| L12 | `mod`/`use`/`pub`, visibility, cross-file modules | **OK** | `crates/xulo-cli/tests/matrix_probes/L12_modules/main.xulo`, `examples/modules`; AOT module build | 1 |
| L13 | Tuple literals, destructuring, equality | **OK** | Literals, `p.i`, destructuring, positional writes, and structural equality for lists, tuples, structs, maps, sets, and enums (`crates/xulo-cli/tests/matrix_probes/L13_tuples.xulo`, `crates/xulo-runtime/tests/equality.rs`, `crates/xulo-compiler/tests/type_system_codegen_tests.rs::structural_equality_compiles`); §6.11 | 1 |
| L14 | Tuple positional access `p.0` and `?.0` | **OK** | Reads and `let mut` writes lower to the tuple's list slot; a position outside the arity is `E0220`; both execution paths agree (`crates/xulo-runtime/tests/type_system.rs`) | 2 |
| L15 | `Array<T, N>` with `.len` and indexed access | **OK** | Interpreter and AOT agree on `.len`, indexing, and iteration; probes cover `Array<int, 3>` and `Array<string, 2>` (`crates/xulo-compiler/tests/type_system_codegen_tests.rs`) | 4 |
| L16 | `T?`, `?.`, `??`, null narrowing | **OK** | `crates/xulo-cli/tests/matrix_probes/L16_optionals.xulo`, `examples/optional_chaining.xulo` | 1 |
| L17 | `unknown` as the top type | **OK** | Every type is assignable to `unknown`; `List<unknown>` and `Map<string, unknown>` accept heterogeneous values. `print` still rejects `unknown` itself (`E0212`, spec `builtins/intrinsic-functions.md`), so a probe narrows first (`crates/xulo-runtime/tests/type_system.rs`). | 2 |
| L18 | `is` / `is not` narrowing in `if` conditions | **OK** | `if v is T` narrows the then-branch and removes covered union members from the else-branch; `is not` swaps them; untestable types are `E0222`. An assignment to the narrowed binding ends the narrowing for the rest of the branch, so the declared type is used afterwards (L18b). `crates/xulo-runtime/tests/type_system.rs` | 2 |
| L19 | `Result`/`Error` and `?` propagation | **OK** | `?` propagation through `Result::Err`; `crates/xulo-cli/tests/matrix_probes/L19_result_question.xulo` | 1 |
| L20 | `defer` runs at block exit in reverse registration order | **OK** | The operand registers in its block's environment without evaluation and runs in reverse at exit; `crates/xulo-runtime/tests/defer.rs` covers nesting, loops, `return`, and `?` | 3 |
| L21 | `copy` / `move` and use-after-move checking | **OK** | Both operators are accepted only as a `let` initializer, a call argument, or a `return` operand (`E0403`); `move` requires a whole frame-owned binding and any later use of it is `E0403`. `crates/xulo-semantic/tests/semantic_tests.rs` (`move_of_a_parameter_is_e0403`, `move_of_a_part_of_a_binding_is_e0403`, `using_a_binding_after_move_is_e0403`, `moving_twice_is_e0403`, `move_outside_let_argument_or_return_is_e0403`, `copy_outside_let_argument_or_return_is_e0403`) | — |
| L22 | Async/await, `spawn`, `lock`, `shared` | **OK** | `crates/xulo-runtime/tests/task_slots.rs`, `crates/xulo-runtime/tests/service.rs`, `examples/async_tasks.xulo`; outside the bootstrap subset | — |
| L23 | Attributes, `#[test]`, `#[derive]`, macros | **OK** | `crates/xulo-parser/tests/attribute_tests.rs`, `crates/xulo-parser/tests/macro_syntax_tests.rs`, `crates/xulo-runtime/tests/macro_system.rs`; outside the bootstrap subset | — |
| L24 | UI/components and `@State`/`@Environment` | **PARTIAL** | UI works in examples; `@Environment` is explicitly unsupported; outside the bootstrap subset | — |
| L25 | Type aliases (`type X = …`) | **OK** | Alias resolution, alias receivers in trait dispatch, and function-type aliases; `crates/xulo-semantic/tests/semantic_tests.rs` (`type_alias_is_resolved`, `trait_dispatch_resolves_type_aliases`, `call_value_alias_aliases_callable`), `crates/xulo-parser/tests/parser_tests.rs` (`type_alias`). Allowed by the subset, not yet used by `xuloc/` | — |

### 5.2 Runtime semantics

| ID | Requirement | Status | Evidence / limitation | Needed by |
|----|-------------|--------|-----------------------|-----------|
| R1 | Deterministic left-to-right evaluation | **OK** | `crates/xulo-cli/tests/matrix_probes/R1_evaluation_order.xulo` | 1 |
| R2 | 64-bit integer arithmetic, overflow/wrapping, shifts | **OK** | Interpreter `Num` tower (`crates/xulo-runtime/src/value.rs`) and AOT runtime helpers `xulo_int_add/sub/mul/div/rem/pow/neg` trap on overflow, division by zero, and a negative exponent; `xulo_shift`/`xulo_num_compare` implement wrap-on-left, sign- or zero-fill right shifts, and exact signed/unsigned comparison; `crates/xulo-runtime/tests/integers.rs` and `crates/xulo-compiler/tests/integer_model_codegen_tests.rs` | 1 |
| R3 | UTF-8 scalar string operations | **OK** | `crates/xulo-cli/tests/matrix_probes/R3_utf8_strings.xulo`; `.len`, `.byte_len`, `.code_at`, `.slice`, `.bytes()` in interpreter and AOT | 1 |
| R4 | Runtime failures with stable codes | **OK** | `crates/xulo-cli/tests/matrix_probes/R4_runtime_codes.xulo` (`E0006` on an out-of-range index) | 1 |
| R5 | No panic on adversarial input; nesting guard | **OK** | `crates/xulo-compiler/tests/robustness.rs`, `crates/xulo-parser/tests/parser_tests.rs` | 1 |
| R6 | `panic`, `assert`, and process exit status | **OK** | `crates/xulo-cli/tests/matrix_probes/R6_panic_assert.xulo`, `crates/xulo-cli/tests/process_exit.rs` | 1 |
| R7 | `defer` executes at scope exit, reverse order | **OK** | `leave_block` drains the scope queue on normal exit and `?` propagation (before `return`/`break`/`continue` pass out); `crates/xulo-runtime/tests/defer.rs` | 3 |
| R8 | Value/reference semantics for `copy`/`move` | **OK** | `copy` deep-copies lists, maps, sets, builders, structs, enums, and tuples, mirrors cycles, and keeps identity-carrying values shared (`crates/xulo-runtime/tests/native.rs::copy_detaches_sets_enums_and_nested_containers`, `crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs::copy_detaches_reference_values_and_keeps_float_slots`); `move` transfers and invalidates the binding (L21). Plain `let`, assignment, destructuring, loop, and match binding now apply the value/reference table: `struct`/`enum`/`Array`/tuple/`Range` values get their own container and `List`/`Map`/`Set`/`StringBuilder` stay shared (`crates/xulo-runtime/tests/value_semantics.rs`, `crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs::bindings_copy_value_aggregates_and_share_references`). | — |
| R9 | Interpreter and AOT produce identical observable behavior | **OK** | `crates/xupm/tests/aot_differential.rs` compares all 30 top-level examples byte for byte, including dynamic JSON index tags, generic bool returns, HTTP server round-trips, process `Command`/`Child` handles, and `Task::resolve`/`all`/`race`/`timeout`; the shared baseline is empty. Compiled async calls run eagerly, so race order and timeout durations follow synchronous completion; that model is part of the AOT backend, not an unrecorded baseline gap. | 4 |
| R10 | Deterministic output formatting for diagnostics and integers | **OK** | `int`/`u64` print and stringify through exact decimal formatting in both paths; `crates/xulo-runtime/tests/integers.rs` covers `u64::MAX`, `2^63`, and `-2^63` | 2 |

### 5.3 Collections and text

| ID | Requirement | Status | Evidence / limitation | Needed by |
|----|-------------|--------|-----------------------|-----------|
| C1 | `List` with `.push`, `.pop`, `.len`, indexing, `+`, iteration | **OK** | Interpreter and AOT agree, including spread; `examples/collections.xulo` matches byte for byte (`crates/xulo-compiler/tests/type_system_codegen_tests.rs`) | 1 |
| C2 | `Map<K, V>` with `.get`, `.set`, `.has`, `.remove`, `.size`, iteration | **OK** | Interpreter and compiled paths agree on members, projection, `Map::from_entries`, typed literals, brace-map spread, and key iteration (`crates/xulo-compiler/tests/type_system_codegen_tests.rs`, `crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs`) | 4 |
| C3 | `Set<T>` with `.insert`, `.remove`, `.has`, `.size` | **OK** | Interpreter and compiled paths agree on construction, membership, insertion, removal, size, and iteration for numbers, strings, and payload-free enums (`crates/xulo-compiler/tests/type_system_codegen_tests.rs`, `crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs`) | 4 |
| C4 | `Array<T, N>` with `.len`, indexing, iteration | **OK** | Interpreter and AOT agree on `.len`, indexing, iteration, and `Array::filled<T, N>` (`crates/xulo-runtime/tests/native.rs`, `crates/xulo-compiler/tests/type_system_codegen_tests.rs`) | 4 |
| C5 | `string` members and UTF-8 behavior | **OK** | `crates/xulo-runtime/tests/string_methods.rs`, `crates/xulo-runtime/tests/string_introspection.rs` | 1 |
| C6 | `StringBuilder` with `new`/`append`/`clear`/`build`/`len`/`byte_len` | **OK** | `crates/xulo-runtime/tests/native.rs` (interpreter and AOT) | 1 |
| C7 | `str`/`fmt`/output of all bootstrap values | **OK** | `str`, `Format`, and `print` render `int`/`u64` exactly (AOT tag 10 for the unsigned slot); `crates/xulo-runtime/tests/integers.rs` and the interpreter/AOT golden suites | 2 |

### 5.4 Standard library and OS services

| ID | Requirement | Status | Evidence / limitation | Needed by |
|----|-------------|--------|-----------------------|-----------|
| S1 | `std::fs` read/write/stat/read_dir/walk/remove/rename/temp | **OK** | `crates/xulo-runtime/tests/fs.rs`, `examples/fs.xulo` | 1 |
| S2 | `std::io` print/eprint/flush/read | **OK** | `crates/xulo-runtime/tests/io.rs` | 1 |
| S3 | `std::process` args/exit/run/exec/launch | **OK** | Interpreter covered by `crates/xulo-runtime/tests/process.rs`; AOT now lowers `launch`/`Command`/`Child` (wait, kill, pid, timeout), and `examples/process.xulo` compares byte for byte in `crates/xupm/tests/aot_differential.rs`. | 1 |
| S4 | `std::path`, `std::os`, environment, cwd, pid | **OK** | `crates/xulo-runtime/tests/os.rs`, `crates/xulo-runtime/tests/path.rs` | 1 |
| S5 | `std::json` parse/stringify/valid | **OK** | `crates/xulo-runtime/tests/json.rs` | 2 |
| S6 | `std::regex` and `std::fmt` | **OK** | `crates/xulo-runtime/tests/regex.rs`, `crates/xulo-runtime/tests/fmt.rs` | 2 |
| S7 | `std::net` and `std::http` | **OK** | `crates/xulo-runtime/tests/net.rs`, `crates/xulo-runtime/tests/http.rs`; outside the bootstrap subset | — |
| S8 | `extern "C"` FFI | **PARTIAL** | Unix only; Windows returns an unsupported error; outside the bootstrap subset | — |
| S9 | Numeric signatures and std diagnostics match the std spec | **OK** | Numeric results name `int`/`float`/`u64`; AOT rounding narrows to the `int` slot with a runtime range failure; regex types annotate without imports and aliases reach their statics; stable diagnostic codes from D3 (`crates/xulo-semantic/tests/semantic_tests.rs`, `crates/xulo-runtime/tests/native.rs`, `crates/xulo-compiler/tests/intrinsics_codegen_tests.rs`) | 2 |
| S10 | `std::math` | **OK** | Interpreter and AOT agree; `crates/xulo-runtime/tests/std_additions.rs`, `crates/xulo-compiler/tests/std_additions_codegen_tests.rs`, `crates/xulo-std/tests/native_semantics.rs`. Allowed by the subset, not yet used by `xuloc/` | — |
| S11 | `std::time` | **OK** | Format, parse, parts, and weekday semantics are tested in the shared `xulo-std` implementation; the AOT path is covered by `examples/time.xulo` in the differential suite (`crates/xulo-std/tests/time_semantics.rs`, `crates/xulo-std/tests/native_semantics.rs`). Allowed by the subset, not yet used by `xuloc/` | — |

### 5.5 Modules, build, and toolchain

| ID | Requirement | Status | Evidence / limitation | Needed by |
|----|-------------|--------|-----------------------|-----------|
| B1 | `xulo run` and `xulo check` for a single file | **OK** | `crates/xulo-cli/tests/integration_tests.rs` | 1 |
| B2 | `xupm build` for core language and `hello` | **OK** | `.github/workflows/ci.yml`: macOS/Windows AOT smoke test | 4 |
| B3 | `xupm build` for the bootstrap subset | **OK** | `crates/xupm/tests/aot_differential.rs` compares all 30 examples and `crates/xupm/tests/cli.rs` covers build-failure exit status. FS bytes, path `split`/`relative`, HTTP server types, `os::exec_path`, process `launch`/`Command`/`Child`, and the `Task` combinators lower and compare. | 4 |
| B4 | AOT module tree (`mod`/`use`) | **OK** | `crates/xulo-compiler/tests/module_system.rs`, `examples/modules` native build | 4 |
| B5 | Runtime static library and linker path | **OK** | `.github/workflows/ci.yml` and the release workflow | 4 |
| B6 | `xupm test` for stage 1's own tests | **OK** | `crates/xupm/tests/cli.rs::runs_stage1_test_file` runs `xuloc/tests.xulo` (10 tests); `crates/xulo-cli/tests/stage1.rs` covers the `xulo test` path | 3 |
| B7 | Linux/macOS/Windows build matrix | **PARTIAL** | Linux and macOS run the full AOT differential; macOS verified locally on 2026-11-17 (30 examples plus the stage 1 workload). Windows now runs the same differential with `crates/xupm/tests/aot_differential_baseline.windows.tsv` (`ffi.xulo`, since FFI is Unix-only, S8), but its first CI run is pending (open B7 in `tasks.md` WP3). | 4 |
| B8 | Reproducible stage 1 build and self-compile smoke test | **PARTIAL** | B8a: `xupm build xuloc/main.xulo --release` produces a native front end that matches the interpreter (`crates/xupm/tests/aot_differential.rs::stage1_binary_matches_the_interpreter`). Reproducibility and the stage 1 self-build await WP6. | 5 |

### 5.6 Diagnostics, testing, and conformance

| ID | Requirement | Status | Evidence / limitation | Needed by |
|----|-------------|--------|-----------------------|-----------|
| D1 | Source spans and `file:line:column` diagnostics | **OK** | `crates/xulo-core/tests/diagnostics_tests.rs`, `crates/xulo-cli/tests/integration_tests.rs` | 1 |
| D2 | stderr writes and nonzero exit status | **OK** | `crates/xulo-cli/tests/process_exit.rs`, `crates/xulo-cli/tests/matrix_probes/R6_panic_assert.xulo` | 1 |
| D3 | Stable error codes for the bootstrap subset | **OK** | Called constants are `E0204`; `std::regex` value paths are `E0607`, unknown module members `E0601`, and an unimported type used as a value `E0101` (std-spec `modules/regex.md`). The head/member diagnostics std-spec §1.3 named as coarse fallbacks now carry their fine-grained codes (`crates/xulo-semantic/tests/semantic_tests.rs`, `crates/xulo-compiler/tests/intrinsics_codegen_tests.rs`). | 2 |
| D4 | Stage 1's own diagnostics are consistent with stage 0 | **OK** | Lexical/syntax use `E0001`/`E0002`; checker failures use `E0101`/`E0201`/`E0202`/`E0207`/`E0211`/`E0213`/`E0215` with `file:line:column`. Stage 0 now reports `E0101` for undefined names, `E0201` for conditions, `E0203` for return mismatches, `E0207` for arity, and `E0213` for non-iterables; `crates/xulo-cli/tests/stage1.rs` compares the codes in `stage1_check_agrees_with_stage0_on_the_corpus`. | 3 |
| T1 | Unit and integration tests | **OK** | `cargo test --workspace`: 2129 passed, 0 failed (2026-11-17) | 1 |
| T2 | All examples run in CI | **OK** | `.github/workflows/ci.yml` examples job | 1 |
| T3 | Pinned-spec example conformance runner | **OK** | `crates/xulo-cli/tests/spec_conformance.rs`: 114 annotated examples extracted from the pinned spec, 81 pass, 33 triaged in `spec_conformance_baseline.tsv` (§6.10) | 2 |
| T4 | Interpreter vs. AOT differential suite | **OK** | `crates/xupm/tests/aot_differential.rs` builds and executes every top-level example, compares stdout/stderr against `xupm run`, and fails on stale baseline entries; CI runs it on Linux and macOS after the release build. The shared baseline is empty; Windows parks `ffi.xulo` in `crates/xupm/tests/aot_differential_baseline.windows.tsv` | 4 |
| T5 | Stage 1 self-parse/self-check/self-build tests | **PARTIAL** | Self-parse, self-check, and interpreter smoke tests are live (`crates/xulo-cli/tests/stage1.rs`, `xuloc/tests.xulo`); self-build awaits WP6 | 2–5 |
| T6 | Robustness/fuzz tests | **OK** | `crates/xulo-compiler/tests/robustness.rs`, `crates/xulo-parser/tests/parser_tests.rs` | 1 |
| T7 | Spec and std-spec submodules aligned with the pin | **OK** | `crates/xulo-cli/tests/submodule_pins.rs` compares the `HEAD` gitlink, the index, and the checked-out submodule against `spec@48f49b7` / `std-spec@a2e4cb7`; CI job `conformance` | 1 |

## 6. Known blocking defects

These are the rows that should be fixed before making a "stage 0 host-ready"
claim. Each probe is intentionally small enough to become a regression test.

### 6.1 64-bit integer model (L2, R2, R10, C7, R9)

```xulo
fn main() {
    print(9007199254740993 == 9007199254740992) // expected: false
    print(1 << 62)                              // expected: 4611686018427387904
    let max: u64 = 18446744073709551615         // expected: accepted
    print(max)
}
```

**Resolved (WP1).** The probe prints `false`,
`4611686018427387904`, and `18446744073709551615` under both `xulo run` and
`xupm build`. `xulo-core` now splits `Literal::Int(i128)` from
`Literal::Float(f64)`; the interpreter's numeric tower carries `Int(i64)`,
`UInt(u64)`, and `Float(f64)` (`crates/xulo-runtime/src/value.rs`); and the
AOT path lowers integer arithmetic to checked `xulo_int_*` runtime helpers and
prints `u64` values with tag `10`. `crates/xulo-runtime/tests/integers.rs`
covers `2^53`, `2^63`, `2^64-1`, underflow/overflow traps, shifts, `to_fixed`,
and the wrapping intrinsics. A literal above `u64::MAX` remains a parse error
(`E0209`).

### 6.2 `unknown` is not a top type (L17)

```xulo
fn show(v: unknown): unknown { return v }
fn main() {
    let v = show("x")
    if v is string {
        print(v) // expected: x
    }
    let xs: List<unknown> = ["a", 1, true]
    print(xs.len) // expected: 3
}
```

**Resolved (WP2).** A `string` now fits an `unknown` parameter/return
position, and `List<unknown>`/`Map<string, unknown>` accept heterogeneous
values. The probe narrows before printing because `print` rejects `unknown`
itself (`E0212`, spec `builtins/intrinsic-functions.md`); the spec's
`unknown` has no string form. Interpreter and AOT agree on the probe.

### 6.3 `is` narrowing is missing (L18)

```xulo
fn f(v: int | string): int {
    if v is int {
        return v + 1 // expected: accepted; v is int here
    }
    return 0
}
```

**Resolved (WP2, L18a).** The then-branch checks with `v : int`; the
else-branch removes the members `T` covers (`string | int` minus `int` is
`string`). `is not T` swaps the branches, `unknown` stays `unknown` in the
negative branch, and an untestable or mismatched tested type is `E0222`.
An assignment to the narrowed binding ends the narrowing for the rest of the
branch (L18b), so a write of another union member is accepted and a later
operation checks against the declared type again.

### 6.4 Tuple positional access is missing (L14)

```xulo
fn main() {
    let p: (int, string) = (10, "ten")
    print(p.0) // expected: 10
}
```

**Resolved (WP2).** `p.0` reads the first element in both paths; `p?.0`
returns `null` for a null optional. `let mut r = …; r.0 = 5` writes through
the mutable binding, and a position at or past the arity is `E0220`.
Destructuring `let (a, b) = p` continues to work.

### 6.5 Generic struct construction does not infer field types (L11)

```xulo
struct Pair<A, B> { first: A, second: B }
fn main() {
    let p = Pair(first: 1, second: "x") // expected: Pair<int, string>
    print(p.first)
}
```

**Resolved (WP2).** The constructor infers `Pair<int, string>` from the named
field arguments, and `p.first`/`p.second` substitute the inferred arguments
into the declared field types. The typed object-literal form still works.

### 6.6 AOT collection support (C2, C3)

```xulo
fn main() {
    let m: Map<string, int> = { a: 1 }
    print(m.get("a"))
    print(m.size)
    let mut s = Set::from_list([1, 2, 2])
    print(s.size)
    print(s.has(2))
}
```

The `Map` and `Set` probes pass under both paths.

### 6.7 AOT and interpreter agree on `Array.len` (R9, L15)

- `Array<int, 3>.len` returns `3` under both paths; indexing and iteration
  also agree.

No known observable divergence remains in the §6.6–6.7 probes.

### 6.8 `defer` has the wrong execution time (L20, R7)

```xulo
fn main() {
    defer print("first")
    defer print("second")
    print("body")
}
```

Actual: `first`, `second`, `body`. Expected: `body`, `second`, `first`.

**Resolved (WP4).** The interpreter now registers the operand without
evaluating it, drains each scope's queue in reverse at block exit, and runs it
before `return`, `break`, `continue`, and `?`. The AOT backend rejects `defer`
as a bootstrap restriction instead of silently ignoring it. Covered by
`crates/xulo-runtime/tests/defer.rs` and
`crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs`.

### 6.9 No automated spec conformance and a possibly stale submodule (T3, T7)

- No test extracts the pinned spec's `// ok` / `// runs:` examples and runs them.
- The working tree can be detached at an older spec commit than the superproject
  pin. `git submodule status` is part of the pre-gate checklist.

**Resolved (WP0).** `crates/xulo-cli/tests/spec_conformance.rs` now extracts
every annotated `xulo` block from the pinned spec and checks or runs it; local
`// runs:` cases under `crates/xulo-cli/tests/runs_cases/` keep the executable
path covered because the pinned spec has none;
`crates/xulo-cli/tests/submodule_pins.rs` fails when the `HEAD` gitlink, the
index, or the checked-out submodule drifts from `spec@48f49b7` /
`std-spec@a2e4cb7`. Both run in the `conformance` CI job. Divergences are
recorded in §6.10 rather than in this section.

### 6.10 Spec examples that still diverge (T3)

`spec_conformance_baseline.tsv` lists each diverging example with a reason. The
list currently holds 33 of 114 annotated examples; 81 pass. Counted from the
baseline reasons, they fall into four groups (11 + 12 + 8 + 2 = 33):

- **Context-dependent examples (11).** The block calls or names a declaration
  that exists only in the surrounding prose (`load_config`, `c_malloc`,
  `first<T>`, `AppState`, `Status`, `User`, `poll`, `fill`, `greet`, …). These
  can never pass unchanged; a conformance corpus needs the missing declarations,
  not a compiler fix. This group includes the two `AppState` examples that
  cannot reach E0410.
- **Module-scope statements (12).** The spec writes expression statements and
  assignments at module scope; stage 0 accepts module-scope `let` but rejects
  module-scope `x = …`, `xs.sort()`, and `assert(…)` with `E0002`. This is a
  parser/top-level-statement gap, not covered elsewhere in this matrix.
- **Syntax gaps (8).** Map literal forms: typed `Map<K, V>{ … }` (two examples,
  one of which cannot reach E0221) and parenthesized `({ a: 1 })`. A `mut`
  parameter in a function with no return annotation (`fn f(mut x: int) { … }`,
  "expected `self`"), five examples, two of which cannot reach E0404. Typed
  map literals lower in the AOT path (C2), but the parser still rejects them.
- **Semantic gaps (2).** Generic inference from the expected type
  (`let ys: List<int> = empty()`), and `#[test]` bodies that `await`.

Ownership is recorded as a **component**, not a person: this project is developed
by agents, so "who fixes it" is better answered by "which crate must change".
The component is also machine-checkable, which is why the table lives here
rather than as extra columns in `spec_conformance_baseline.tsv` (the runner
parses that file as `<case id>\t<reason>`, and the assignment is per group, not
per case).

| Group | Cases | Owner (component) | Cleared by |
|---|---:|---|---|
| Context-dependent examples | 11 | `crates/xulo-cli/tests/` (self-contained corpus) | Gate 6 |
| Module-scope statements | 12 | `crates/xulo-parser` (top-level statements) | Gate 6 |
| Syntax gaps | 8 | `crates/xulo-parser` (map literals, `mut` parameters) | Gate 5 |
| Semantic gaps | 2 | `crates/xulo-semantic` (inference, `await` in tests) | Gate 5 |

"Cleared by" is the gate whose exit criterion requires the group to be gone;
there is no separate deadline. The syntax and semantic groups are assigned to
Gate 5 because they affect language forms the bootstrap subset allows (typed
map literals, `mut` parameters, expected-type inference), which stage 1 and
stage 2 must handle identically. The module-scope group is different: a
module's top level holds declarations, stage 1's own source never executes
statements there, so the gap does not block self-hosting and belongs to Gate 6
(conformance). The context-dependent group is also Gate 6, because those blocks
can never pass unchanged and need a self-contained corpus.

`crates/xulo-cli/tests/readiness_matrix.rs` fails when a row names a component
path that does not exist. That check only proves the owner is real; that the
group actually shrinks is proven by `spec_conformance` losing baseline lines at
Gate 5/Gate 6. These groups are also candidate rows for the capability matrix;
the harness exists so that removing a baseline line is the visible proof that a
fix landed.

### 6.11 Structural equality for aggregates (L13, R9)

```xulo
struct P { x: int, y: int }
fn main() {
    print(P(x: 1, y: 2) == P(x: 1, y: 2)) // expected: true
    print([1, 2] == [1, 2])                   // expected: true
    print((1, 2) == (1, 2))                   // expected: true
}
```

**Resolved.** The specification (`expressions/operators.md`, "Equality")
requires structural equality for every value-carrying type. The interpreter's
`equal` now recurses through lists, sets, objects (structs and maps), and enum
payloads, comparing map/struct entries by key and sets without regard to
insertion order. The AOT path adds the runtime
`xulo_value_equal(left, left_tag, right, right_tag)` helper with the same
semantics and a visited-pair guard, and the IR lowering routes structural
`==`/`!=` to it instead of a machine-word handle comparison. The §6.11 probe
passes in both paths. Evidence:
`crates/xulo-runtime/tests/equality.rs`,
`crates/xulo-compiler/tests/type_system_codegen_tests.rs::structural_equality_compiles`,
and the updated `crates/xulo-cli/tests/matrix_probes/L13_tuples.xulo`.

## 7. Work packages and recommended order

### WP0 — Matrix hygiene and conformance (P0, Gate 1/2)

- Add a CI check that `git submodule status` matches the pinned commits.
- Add a conformance runner that extracts `xulo` blocks with `// ok`, `// error[E…]`,
  and `// runs:` from the pinned spec and runs them through stage 0.
- Keep every "OK" row in this document backed by a test name or a probe path.

**Exit:** conformance job runs in CI and its failures are triaged in this
matrix.

**Status:** done for Gates 1–2. T3c's `runs:` path is covered by
`crates/xulo-cli/tests/runs_cases/basics.md`, which the conformance runner scans
alongside the pinned spec. The CI job is `conformance`; §6.10 is the triage, and
`crates/xulo-cli/tests/readiness_matrix.rs` enforces that every OK row cites an
existing test/probe, runs the `matrix_probes` corpus, and fails on an uncited
probe. The audit itself fixed two small defects (implicit return of a trailing
struct construction, tuple destructuring of a tuple type) and re-graded L13 to
PARTIAL for identity-based equality (§6.11, since resolved by L13b).

### WP1 — 64-bit integer model (P0, Gate 1/2/5)

- Split integer and floating literals in `xulo-core` (`Literal::Int(i128)` and
  `Literal::Float(f64)`).
- Split integer and floating runtime values in `xulo-runtime` (`Value::Int`,
  `Value::UInt`, `Value::Float`) and update arithmetic, comparison, formatting,
  and FFI.
- Parse `u64` literals without an `f64` round trip; support `2^64-1`.
- Make interpreter and AOT agree on overflow, wrapping, shifts, and printing.
- Add edge tests: `2^53`, `2^63`, `2^64-1`, `0 - 1`, `1 << 63`, `to_fixed`,
  and wrapping intrinsics.

**Exit:** §6.1 probe passes in both `xulo run` and `xupm build`.

**Status:** done. See §6.1 and `crates/xulo-runtime/tests/integers.rs`.

### WP2 — Type system and semantic convergence (P0, Gate 2/3)

- Implement `unknown` as the top type, including `List<unknown>`,
  `Map<string, unknown>`, and parameter/return positions.
- Implement `is` / `is not` branch narrowing, including assignment ending the
  narrowing, as specified in `expressions/control-flow.md`.
- Implement tuple positional access `p.0` / `p?.0` and writes through `let mut`.
- Implement generic struct construction from named arguments and expected type.
- Add regression tests lifted from the pinned spec.

**Exit:** §6.2–6.5 probes pass; no workaround needed for stage 1's AST.

**Status:** complete (12 of 12). §6.2–6.5 pass in both execution paths, L18b is
covered by the semantic and interpreter regression tests, D3's fine-grained
diagnostic codes are asserted in the semantic tests, S9 aligns the std numeric
result types, AOT narrowing slot, range failures, and regex reachability with
`std-spec` §1.3, L13b makes aggregate `==` structural in both execution paths,
and L6 treats an unannotated lambda's `any` parameter as a unification
wildcard so a later argument can fix the callback's type parameter.

### WP3 — AOT parity and bootstrap backend (P1, Gate 4)

- AOT runtime support for `Map` and `Set` is in place (C2, C3).
- Known AOT parity probes now agree, including spread, ternaries,
  `Array.len`, and `Array::filled` (C1, R9b, L15, C4).
- The differential runner is in place and compares all 30 examples; the shared
  baseline is empty. Windows parks `ffi.xulo`
  in `crates/xupm/tests/aot_differential_baseline.windows.tsv` (S8). FS bytes,
  path `split`/`relative`, HTTP server types, `os::exec_path`, process
  `launch`/`Command`/`Child`, and the `Task` combinators are lowered (B3c, S3a,
  R9c).
- `xupm build` failures return a nonzero exit status and are covered by
  `crates/xupm/tests/cli.rs` (B3a).

**Exit:** §6.6–6.7 probes pass; stage 1 can be built by `xupm` and matches
`xulo run` on the bootstrap workload (Gate 4, reached), and every top-level
example compares byte for byte.

### WP4 — `defer` semantics (P1, Gate 3)

- Implement a defer queue per block in the interpreter: register without
  evaluating, run at scope exit in reverse order, and run before returning,
  breaking, or propagating `?`.
- Decide whether the AOT path supports `defer`; if not, state it as a bootstrap
  restriction rather than silently executing it immediately.

**Exit:** §6.8 probe passes; stage 1 can use `defer` for cleanup.

**Status:** done (L20a, L20b, R7, B3b, G4). The AOT backend reports a clear
error for `defer` rather than dropping the operand.

### WP5 — Stage 1 skeleton (can start in parallel)

- Create `xuloc/` (or the agreed location) with a lexer and parser written in
  Xulo, run by `xulo run`.
- Start with the subset rows marked OK; record every PARTIAL workaround in
  `xuloc/BOOTSTRAP.md`.
- First smoke test: stage 1 parses a fixed example and prints spans/tokens.
- Second smoke test: stage 1 parses its own source.

**Exit:** Gate 1 and Gate 2 — reached on 2026-11-16.

**Status:** done. `xuloc/` holds the generic AST, lexer, parser, first checker,
CLI, corpus, and self-tests. Gate 1 is covered by the token/parse smoke tests and the clean
diagnostic test; Gate 2 self-parses all six stage 1 files, discovered from
`xuloc/*.xulo` by the test. `xulo test` and `xupm test` both run
`xuloc/tests.xulo` (10 tests).

### WP6 — Self-host loop (after WP1–WP5)

- Stage 1 checks and compiles a fixed bootstrap corpus.
- Stage 0 builds stage 1 with `xupm`; stage 1 builds itself; compare stage 1 and
  stage 2.
- Add the bootstrap suite to CI, alongside the conformance runner.

**Exit:** Gate 4–6.

**Status:** in progress. B8a, D4, Gate 3, and Gate 4 are done. `xuloc` now has a
checked bootstrap corpus whose diagnostics match stage 0's codes, and a native
front end whose `tokens`/`parse`/`check` output matches the interpreter. The
`conformance` CI job now runs the bootstrap suite beside the pinned-spec
runner. Gate 5 and the code-generation/self-build items remain open. The native build surfaced and fixed three stage 0 defects:
unreachable IR after a terminating block, pointer-based `string` equality in
the compiled path, and user methods named like string members being dispatched
to the built-in lowering.

### WP7 — Non-gating follow-ups (no gate)

- Triage the 33 spec-conformance divergences by group with an owner and the
  gate that clears it per group (T3f). Gate 6 requires this.
- `copy`/`move` and use-after-move checking (L21) and value/reference binding
  semantics (R8) are done. Plain `let`/assignment copies value-category
  aggregates while reference values keep sharing; `xuloc` can rely on both.

Items that gate work depends on are kept in their work packages (R9c and B7 in
WP3).

**Status:** T3f is done: §6.10 records the group counts, an owner component, and
the gate that clears each group, and `readiness_matrix` checks that every owner
path exists, and T3c's local `// runs:` cases keep the executable path covered.
L21 and R8 are done and non-gating.

## 8. Non-goals for stage 1 bootstrap

The following are deliberately out of scope until the bootstrap loop works:

- UI components, renderers, `@State`/`@Environment`, and the terminal/webview
  backends.
- LSP, IDE integration, and the formatter's comment handling.
- Async tasks, `spawn`/`lock`/`shared`, networking (`std::net`, `std::http`),
  and package registry work.
- Macros and attribute-macro expansion, except for `#[test]` in stage 1's own
  tests.
- Full FFI and Windows FFI; a stage 1 compiler should use `std::fs`/`std::io`/
  `std::process` instead.

These features can remain P2 in the matrix. They become relevant only when
stage 1 needs to compile a program that uses them, not to bootstrap the
compiler itself.

## 9. Verification commands

Use these when updating the matrix:

```bash
# Full workspace tests; record the count and date.
cargo test --workspace

# WP0 gates: pinned submodules and pinned-spec examples.
cargo test -p xulo-cli --test submodule_pins --test spec_conformance -- --nocapture

# WP0/T3d + §6.10: every OK row cites a live test/probe, and every
# spec-divergence group names an existing component path.
cargo test -p xulo-cli --test readiness_matrix -- --nocapture

# WP5/Gate 3: stage 1 runs under the interpreter, parses, and checks the corpus.
cargo test -p xulo-cli --test stage1
cargo test -p xupm --test cli runs_stage1_test_file

# Single-file behavior under the interpreter.
target/debug/xulo check path/to/probe.xulo
target/debug/xulo run path/to/probe.xulo

# Native path (requires the runtime static library).
XULO_RUNTIME_LIB="$PWD/target/release/libxulo_runtime.a" \
  target/release/xupm build path/to/probe.xulo --release

# Gate 4/B8a: stage 1 native output matches the interpreter.
XULO_AOT_DIFFERENTIAL=1 \
XULO_RUNTIME_LIB="$PWD/target/release/libxulo_runtime.a" \
  cargo test -p xupm --test aot_differential stage1_binary_matches_the_interpreter

# Repository state.
git submodule status
git ls-tree HEAD spec std-spec
```

Last recorded verification (2026-11-17):

| Check | Result |
|-------|--------|
| `cargo test --workspace` | 2129 passed, 0 failed |
| Examples (CI job) | Configured to run every `examples/*.xulo`; `async_tasks` uses `u64` sleep input |
| AOT smoke tests | `hello` on macOS/Windows; `structs`, `trait_shapes`, `generics`, `modules` probed locally |
| AOT differential suite | 30 examples compare byte for byte; the shared baseline is empty, and Windows parks only `ffi.xulo` in `crates/xupm/tests/aot_differential_baseline.windows.tsv` |
| AOT build failure | `xupm build` exits nonzero and reports the diagnostic (`crates/xupm/tests/cli.rs`) |
| Stage 1 smoke tests | `cargo test -p xulo-cli --test stage1` — 7 passed; self-parses `xuloc/*.xulo`, clean diagnostics, `xulo test` |
| Stage 1 under `xupm test` | `cargo test -p xupm --test cli runs_stage1_test_file` — 10 stage 1 tests passed |
| Stage 1 native build | `cargo test -p xupm --test aot_differential stage1_binary_matches_the_interpreter` (with `XULO_AOT_DIFFERENTIAL=1`) — interpreter and native output agree |
| Stage 1 checker | `cargo test -p xulo-cli --test stage1 stage1_check_agrees_with_stage0_on_the_corpus` — 2 accept + 6 reject files agree with stage 0 |

## 10. Maintenance

- Update this document when a capability changes status; do not let a row stay
  "OK" after a regression.
- A row may move to "OK" only with a test or a named probe, and the probe should
  live in a `tests/` directory rather than only in this document.
- At each gate review, record the date, the stage 1 commit, the stage 0 commit,
  and the pinned spec commits.
- If the bootstrap subset changes, update §2.1 and the affected rows in the
  same change.
- Treat this document as the stage 1 definition-of-done, not as a second
  language specification. When it disagrees with `spec/` or `std-spec/`, the
  pinned specification wins.
