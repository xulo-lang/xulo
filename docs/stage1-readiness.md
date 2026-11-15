# Stage 1 Bootstrap Readiness

- **Status**: Living document
- **Date**: 2026-11-15
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
`stage1/BOOTSTRAP.md`.

**Current assessment:** **Achievable now.** The interpreter is sufficient for a
first lexer/parser if the stage 1 code avoids the P0 gaps in §5. This is the
recommended starting point.

### Gate 2 — Self-parse

**Exit criterion:** stage 1 parses its own source (or a fixed, nontrivial
bootstrap file) and reports accurate spans. Round-trip or structural checks
cover literals, expressions, declarations, and modules.

**Current assessment:** **At risk.** Gate 2 itself is reachable, but a
spec-shaped parser wants `int` for positions, tuples for spans, `unknown` for
heterogeneous AST fields, and `is` narrowing for node categories. Workarounds
exist, but they should be written down rather than discovered late.

### Gate 3 — Self-check

**Exit criterion:** stage 1 type-checks a nontrivial bootstrap file and agrees
with stage 0 on accept/reject for the bootstrap subset. Diagnostics include
file/line/column and exit nonzero on failure.

**Current assessment:** **Blocked by type-system gaps.** `unknown`, `is`
narrowing, tuple positional access, and generic struct construction are the
main risks. Fixing P0 rows in §5 is the shortest path.

### Gate 4 — Native build

**Exit criterion:** `xupm build` produces a stage 1 binary that runs the same
bootstrap workload as `xulo run` and prints identical output.

**Current assessment:** **Blocked by AOT gaps.** `Map` now lowers through the
compiled path, but `Set` is still missing there; list spread, nested ternaries,
and `Array.len` have known divergences. The interpreter can be used for
Gate 1–3, but native bootstrap requires these fixes or an explicitly narrower
stage 1 design.

### Gate 5 — Self-host

**Exit criterion:** stage 1 compiles itself, producing a stage 2 binary; stage 1
and stage 2 agree on the bootstrap suite and on the conformance subset. A
failure in either direction is a release blocker.

**Current assessment:** **Blocked by numeric semantics and conformance.** The
64-bit integer model must be fixed first; otherwise spans, hashes, and bit
operations are not trustworthy across interpreters and backends.

### Gate 6 — Conformance

**Exit criterion:** the pinned specification's checkable examples and the
bootstrap suite run in CI; stage 1 and stage 0 agree on the bootstrap subset;
known deviations are listed with an owner and a deadline.

**Current assessment:** **No runner yet.** This document's matrix is the
interim checklist; it is not a substitute for automated conformance.

## 5. Capability matrix

### 5.1 Language and type system

Priority is relative to Gate 1/2 unless stated otherwise.

| ID | Requirement | Status | Evidence / limitation | Needed by |
|----|-------------|--------|-----------------------|-----------|
| L1 | UTF-8 lexing and parsing of `.xulo` files | **OK** | `xulo-parser` tests and all examples | 1 |
| L2 | `int` is a 64-bit signed integer; `u64` covers `0…2^64-1` | **OK** | `Literal::Int(i128)` + `Literal::Float(f64)`; `Value::Int(i64)`/`UInt(u64)`/`Float(f64)`; integers are exact above `2^53` and through `u64::MAX`; a literal above the `u64` range is `E0209`; `crates/xulo-runtime/tests/integers.rs` | 1 |
| L3 | `float`/`f64` arithmetic | **OK** | `f64` runtime; examples and tests | 1 |
| L4 | `let`, `let mut`, assignment, shadowing | **OK** | semantic/runtime tests | 1 |
| L5 | Functions, recursion, implicit return | **OK** | examples (`fibonacci`, `closures_hof`) | 1 |
| L6 | Closures and function values | **PARTIAL** | Named-function callbacks work; an untyped lambda passed to a generic callback can fail with `E0202` | 2 |
| L7 | `if`/`else`, `while`, `for`, `match`, `break`, `continue` | **OK** | examples and tests | 1 |
| L8 | Enums, payloads, recursive/generic enums, exhaustive `match` | **OK** | `enums_match`, probes with `Tree<T>` | 1 |
| L9 | Structs, `impl`, traits, explicit `Trait::method(recv)` dispatch | **OK** | `structs`, `trait_shapes` | 1 |
| L10 | Generic functions, bounds, `where` clauses | **OK** | `generics`, `T: Show` probe | 2 |
| L11 | Generic struct construction with named arguments | **OK** | The constructor infers type arguments from named field arguments and member access substitutes them; `crates/xulo-runtime/tests/type_system.rs`, `crates/xulo-compiler/tests/type_system_codegen_tests.rs` | 2 |
| L12 | `mod`/`use`/`pub`, visibility, cross-file modules | **OK** | `examples/modules`; AOT module build | 1 |
| L13 | Tuple literals, destructuring, equality | **OK** | parser/semantic/runtime tests | 1 |
| L14 | Tuple positional access `p.0` and `?.0` | **OK** | Reads and `let mut` writes lower to the tuple's list slot; a position outside the arity is `E0220`; both execution paths agree | 2 |
| L15 | `Array<T, N>` with `.len` and indexed access | **PARTIAL** | Interpreter works; AOT `.len` returns the wrong value | 4 |
| L16 | `T?`, `?.`, `??`, null narrowing | **OK** | `optional_chaining`, semantic narrowing tests | 1 |
| L17 | `unknown` as the top type | **OK** | Every type is assignable to `unknown`; `List<unknown>` and `Map<string, unknown>` accept heterogeneous values. `print` still rejects `unknown` itself (`E0212`, spec `builtins/intrinsic-functions.md`), so a probe narrows first. | 2 |
| L18 | `is` / `is not` narrowing in `if` conditions | **OK** | `if v is T` narrows the then-branch and removes covered union members from the else-branch; `is not` swaps them; untestable types are `E0222`. An assignment to the narrowed binding ends the narrowing for the rest of the branch, so the declared type is used afterwards (L18b). | 2 |
| L19 | `Result`/`Error` and `?` propagation | **OK** | tests and `std` examples | 1 |
| L20 | `defer` runs at block exit in reverse registration order | **BROKEN** | Runtime executes the operand immediately; `defer` prints in wrong order | 3 |
| L21 | `copy` / `move` and use-after-move checking | **PARTIAL** | Syntax/checker present; full runtime and move-invalidation behavior not verified here | 3 |
| L22 | Async/await, `spawn`, `lock`, `shared` | **OK** | Runtime tests/examples; outside the bootstrap subset | — |
| L23 | Attributes, `#[test]`, `#[derive]`, macros | **OK** | parser/semantic/runtime tests; outside the bootstrap subset | — |
| L24 | UI/components and `@State`/`@Environment` | **PARTIAL** | UI works in examples; `@Environment` is explicitly unsupported; outside the bootstrap subset | — |

### 5.2 Runtime semantics

| ID | Requirement | Status | Evidence / limitation | Needed by |
|----|-------------|--------|-----------------------|-----------|
| R1 | Deterministic left-to-right evaluation | **OK** | specification and runtime tests | 1 |
| R2 | 64-bit integer arithmetic, overflow/wrapping, shifts | **OK** | Interpreter `Num` tower (`crates/xulo-runtime/src/value.rs`) and AOT runtime helpers `xulo_int_add/sub/mul/div/rem/pow/neg` trap on overflow, division by zero, and a negative exponent; `xulo_shift`/`xulo_num_compare` implement wrap-on-left, sign- or zero-fill right shifts, and exact signed/unsigned comparison; `crates/xulo-runtime/tests/integers.rs` and `crates/xulo-compiler/tests/integer_model_codegen_tests.rs` | 1 |
| R3 | UTF-8 scalar string operations | **OK** | `.len`, `.byte_len`, `.code_at`, `.slice`, `.bytes()` in interpreter and AOT | 1 |
| R4 | Runtime failures with stable codes | **OK** | `E0006`, out-of-range tests | 1 |
| R5 | No panic on adversarial input; nesting guard | **OK** | `tests/robustness.rs` | 1 |
| R6 | `panic`, `assert`, and process exit status | **OK** | CLI/runtime tests | 1 |
| R7 | `defer` executes at scope exit, reverse order | **BROKEN** | Same defect as L20 | 3 |
| R8 | Value/reference semantics for `copy`/`move` | **PARTIAL** | Not fully verified; avoid relying on it until tested | 3 |
| R9 | Interpreter and AOT produce identical observable behavior | **PARTIAL** | Integer arithmetic, overflow, shifts, comparison, and printing now agree (`u64` uses print tag 10; §6.1 passes under `xupm build`). List spread, nested ternaries, and `Array.len` still differ (§6.7, WP3). | 4 |
| R10 | Deterministic output formatting for diagnostics and integers | **OK** | `int`/`u64` print and stringify through exact decimal formatting in both paths; `crates/xulo-runtime/tests/integers.rs` covers `u64::MAX`, `2^63`, and `-2^63` | 2 |

### 5.3 Collections and text

| ID | Requirement | Status | Evidence / limitation | Needed by |
|----|-------------|--------|-----------------------|-----------|
| C1 | `List` with `.push`, `.pop`, `.len`, indexing, `+`, iteration | **PARTIAL** | Interpreter OK; AOT list spread produces wrong elements | 1 |
| C2 | `Map<K, V>` with `.get`, `.set`, `.has`, `.remove`, `.size`, iteration | **OK** | Interpreter and compiled paths agree on members, projection, `Map::from_entries`, typed literals, and key iteration (`crates/xulo-compiler/tests/type_system_codegen_tests.rs`, `crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs`) | 4 |
| C3 | `Set<T>` with `.insert`, `.remove`, `.has`, `.size` | **PARTIAL** | Interpreter OK; AOT has no set runtime support | 4 |
| C4 | `Array<T, N>` with `.len`, indexing, iteration | **PARTIAL** | Interpreter OK; AOT `.len` wrong; `Array::filled` not verified | 4 |
| C5 | `string` members and UTF-8 behavior | **OK** | Interpreter and AOT string member tests | 1 |
| C6 | `StringBuilder` with `new`/`append`/`clear`/`build`/`len`/`byte_len` | **OK** | runtime/AOT tests | 1 |
| C7 | `str`/`fmt`/output of all bootstrap values | **OK** | `str`, `Format`, and `print` render `int`/`u64` exactly (AOT tag 10 for the unsigned slot); `crates/xulo-runtime/tests/integers.rs` and the interpreter/AOT golden suites | 2 |

### 5.4 Standard library and OS services

| ID | Requirement | Status | Evidence / limitation | Needed by |
|----|-------------|--------|-----------------------|-----------|
| S1 | `std::fs` read/write/stat/read_dir/walk/remove/rename/temp | **OK** | runtime tests and examples | 1 |
| S2 | `std::io` print/eprint/flush/read | **OK** | runtime tests | 1 |
| S3 | `std::process` args/exit/run/exec/launch | **OK** | runtime tests; AOT symbols present | 1 |
| S4 | `std::path`, `std::os`, environment, cwd, pid | **OK** | runtime tests | 1 |
| S5 | `std::json` parse/stringify/valid | **OK** | runtime tests | 2 |
| S6 | `std::regex` and `std::fmt` | **OK** | runtime tests | 2 |
| S7 | `std::net` and `std::http` | **OK** | runtime tests; outside the bootstrap subset | — |
| S8 | `extern "C"` FFI | **PARTIAL** | Unix only; Windows returns an unsupported error; outside the bootstrap subset | — |
| S9 | Numeric signatures and std diagnostics match the std spec | **OK** | Numeric results name `int`/`float`/`u64`; AOT rounding narrows to the `int` slot with a runtime range failure; regex types annotate without imports and aliases reach their statics; stable diagnostic codes from D3 (`crates/xulo-semantic/tests/semantic_tests.rs`, `crates/xulo-runtime/tests/native.rs`, `crates/xulo-compiler/tests/intrinsics_codegen_tests.rs`) | 2 |

### 5.5 Modules, build, and toolchain

| ID | Requirement | Status | Evidence / limitation | Needed by |
|----|-------------|--------|-----------------------|-----------|
| B1 | `xulo run` and `xulo check` for a single file | **OK** | CLI integration tests | 1 |
| B2 | `xupm build` for core language and `hello` | **OK** | macOS/Windows CI AOT smoke test | 4 |
| B3 | `xupm build` for the bootstrap subset | **BROKEN** | `Map`/`Set` unsupported; spread/ternary/Array bugs | 4 |
| B4 | AOT module tree (`mod`/`use`) | **OK** | `examples/modules` native build | 4 |
| B5 | Runtime static library and linker path | **OK** | CI and release workflow | 4 |
| B6 | `xupm test` for stage 1's own tests | **NOT VERIFIED** | Command exists; no stage 1 test corpus yet | 3 |
| B7 | Linux/macOS/Windows build matrix | **PARTIAL** | Release artifacts exist; Windows FFI is unsupported | 4 |
| B8 | Reproducible stage 1 build and self-compile smoke test | **MISSING** | No stage 1 build in CI | 5 |

### 5.6 Diagnostics, testing, and conformance

| ID | Requirement | Status | Evidence / limitation | Needed by |
|----|-------------|--------|-----------------------|-----------|
| D1 | Source spans and `file:line:column` diagnostics | **OK** | stage 0 diagnostics; tests | 1 |
| D2 | stderr writes and nonzero exit status | **OK** | CLI/runtime tests | 1 |
| D3 | Stable error codes for the bootstrap subset | **OK** | Called constants are `E0204`; `std::regex` value paths are `E0607`, unknown module members `E0601`, and an unimported type used as a value `E0101` (std-spec `modules/regex.md`). The head/member diagnostics std-spec §1.3 named as coarse fallbacks now carry their fine-grained codes. | 2 |
| D4 | Stage 1's own diagnostics are consistent with stage 0 | **NOT VERIFIED** | Requires stage 1 implementation | 3 |
| T1 | Unit and integration tests | **OK** | `cargo test --workspace`: 2019 passed, 0 failed (2026-11-15) | 1 |
| T2 | All examples run in CI | **OK** | `.github/workflows/ci.yml` examples job | 1 |
| T3 | Pinned-spec example conformance runner | **OK** | `crates/xulo-cli/tests/spec_conformance.rs`: 114 annotated examples extracted from the pinned spec, 81 pass, 33 triaged in `spec_conformance_baseline.tsv` (§6.10) | 2 |
| T4 | Interpreter vs. AOT differential suite | **PARTIAL** | LLVM-only golden/differential tests exist; not a general suite | 4 |
| T5 | Stage 1 self-parse/self-check/self-build tests | **MISSING** | No stage 1 code yet | 2–5 |
| T6 | Robustness/fuzz tests | **OK** | `tests/robustness.rs` | 1 |
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

### 6.6 AOT does not support `Set` (C3, B3)

```xulo
fn main() {
    let m: Map<string, int> = { a: 1 }
    print(m.get("a"))
    print(m.size)
}
```

The `Map` probe now passes under both paths (C2). `Set::from_list` still fails
under `xupm build`; the interpreter supports it.

### 6.7 AOT and interpreter diverge on existing examples (R9, C1, L15)

- `examples/collections.xulo`: list spread `...` prints `[0, 0]` under AOT
  instead of `[1, 2, 3, 4]`; `[0, 9]` instead of `[1, 2, 3, 4, 9]`.
- `examples/optional_chaining.xulo`: the nested ternary returns `0` under AOT
  instead of `"B"`.
- `Array<int, 3>.len` returns `1` under AOT instead of `3`.

These are observable differences on programs the interpreter already accepts;
a native stage 1 would inherit them.

### 6.8 `defer` has the wrong execution time (L20, R7)

```xulo
fn main() {
    defer print("first")
    defer print("second")
    print("body")
}
```

Actual: `first`, `second`, `body`. Expected: `body`, `second`, `first`.

### 6.9 No automated spec conformance and a possibly stale submodule (T3, T7)

- No test extracts the pinned spec's `// ok` / `// runs:` examples and runs them.
- The working tree can be detached at an older spec commit than the superproject
  pin. `git submodule status` is part of the pre-gate checklist.

**Resolved (WP0).** `crates/xulo-cli/tests/spec_conformance.rs` now extracts
every annotated `xulo` block from the pinned spec and checks or runs it;
`crates/xulo-cli/tests/submodule_pins.rs` fails when the `HEAD` gitlink, the
index, or the checked-out submodule drifts from `spec@48f49b7` /
`std-spec@a2e4cb7`. Both run in the `conformance` CI job. Divergences are
recorded in §6.10 rather than in this section.

### 6.10 Spec examples that still diverge (T3)

`spec_conformance_baseline.tsv` lists each diverging example with a reason. The
list currently holds 33 of 114 annotated examples; 81 pass. They are not one
defect but four groups:

- **Context-dependent examples (16).** The block calls or names a declaration
  that exists only in the surrounding prose (`load_config`, `c_malloc`,
  `first<T>`, `AppState`, `Status`, …). These can never pass unchanged; a
  conformance corpus needs the missing declarations, not a compiler fix.
- **Module-scope statements (11).** The spec writes expression statements and
  assignments at module scope; stage 0 accepts module-scope `let` but rejects
  module-scope `x = …`, `xs.sort()`, and `assert(…)` with `E0002`. This is a
  parser/top-level-statement gap, not covered elsewhere in this matrix.
- **Syntax gaps (4).** Parenthesized map literals `({ a: 1 })` and a `mut`
  parameter in a function with no return annotation
  (`fn f(mut x: int) { … }`, "expected `self`"). Typed map literals
  `Map<K, V>{ … }` now lower in the AOT path (C2).
- **Semantic gaps (1+).** Generic inference from the expected type
  (`let ys: List<int> = empty()`), `#[test]` bodies that `await`, and
  borrow-checker examples (`E0404`, `E0410`, `E0411`) whose construct is
  rejected earlier for another reason.

The groups above are candidate rows for the capability matrix; the harness
exists so that removing a baseline line is the visible proof that a fix
landed.

## 7. Work packages and recommended order

### WP0 — Matrix hygiene and conformance harness (P0, Gate 1/2)

- Add a CI check that `git submodule status` matches the pinned commits.
- Add a conformance runner that extracts `xulo` blocks with `// ok`, `// error[E…]`,
  and `// runs:` from the pinned spec and runs them through stage 0.
- Keep every "OK" row in this document backed by a test name or a probe path.

**Exit:** conformance job runs in CI and its failures are triaged in this
matrix.

**Status:** done except the row-by-row "OK" audit (T3d). The CI job is
`conformance`; §6.10 is the triage.

### WP1 — 64-bit integer model (P0, Gate 1/2/5)

- Split integer and floating literals in `xulo-core` (`Literal::Number(f64)` is
  the root).
- Split integer and floating runtime values in `xulo-runtime` (or add an
  integer variant) and update arithmetic, comparison, formatting, and FFI.
- Parse `u64` literals without an `f64` round trip; support `2^64-1`.
- Make interpreter and AOT agree on overflow, wrapping, shifts, and printing.
- Add edge tests: `2^53`, `2^63`, `2^64-1`, `0 - 1`, `1 << 63`, `to_fixed`,
  and wrapping intrinsics.

**Exit:** §6.1 probe passes in both `xulo run` and `xupm build`.

**Status:** done. See §6.1 and `crates/xulo-runtime/tests/integers.rs`.

### WP2 — Type-system completeness (P0, Gate 2/3)

- Implement `unknown` as the top type, including `List<unknown>`,
  `Map<string, unknown>`, and parameter/return positions.
- Implement `is` / `is not` branch narrowing, including assignment ending the
  narrowing, as specified in `expressions/control-flow.md`.
- Implement tuple positional access `p.0` / `p?.0` and writes through `let mut`.
- Implement generic struct construction from named arguments and expected type.
- Add regression tests lifted from the pinned spec.

**Exit:** §6.2–6.5 probes pass; no workaround needed for stage 1's AST.

**Status:** done. §6.2–6.5 pass in both execution paths, L18b is covered by the
semantic and interpreter regression tests, D3's fine-grained diagnostic codes
are asserted in the semantic tests, and S9 aligns the std numeric result types,
AOT narrowing slot, range failures, and regex reachability with `std-spec`
§1.3.

### WP3 — AOT parity and bootstrap backend (P1, Gate 4)

- Add AOT runtime support for `Map` and `Set`, or explicitly forbid them in the
  bootstrap subset and provide a supported alternative.
- Fix list spread, nested ternaries, and `Array.len` in the AOT lowering.
- Add a differential runner that builds every non-interactive example both ways
  and compares output.
- Make `xupm build` failures return a nonzero exit status (verify separately;
  several probes currently report errors while a shell pipeline hides the exit
  code).

**Exit:** §6.6–6.7 probes pass; stage 1 can be built by `xupm` and matches
`xulo run` on the bootstrap suite.

### WP4 — `defer` semantics (P1, Gate 3)

- Implement a defer queue per block in the interpreter: register without
  evaluating, run at scope exit in reverse order, and run before returning,
  breaking, or propagating `?`.
- Decide whether the AOT path supports `defer`; if not, state it as a bootstrap
  restriction rather than silently executing it immediately.

**Exit:** §6.8 probe passes; stage 1 can use `defer` for cleanup.

### WP5 — Stage 1 skeleton (can start in parallel)

- Create `stage1/` (or the agreed location) with a lexer and parser written in
  Xulo, run by `xulo run`.
- Start with the subset rows marked OK; record every PARTIAL workaround in
  `stage1/BOOTSTRAP.md`.
- First smoke test: stage 1 parses a fixed example and prints spans/tokens.
- Second smoke test: stage 1 parses its own source.

**Exit:** Gate 1 and Gate 2.

### WP6 — Self-host loop (after WP1–WP5)

- Stage 1 checks and compiles a fixed bootstrap corpus.
- Stage 0 builds stage 1 with `xupm`; stage 1 builds itself; compare stage 1 and
  stage 2.
- Add the bootstrap suite to CI, alongside the conformance runner.

**Exit:** Gate 4–6.

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

# Single-file behavior under the interpreter.
target/debug/xulo check path/to/probe.xulo
target/debug/xulo run path/to/probe.xulo

# Native path (requires the runtime static library).
XULO_RUNTIME_LIB="$PWD/target/release/libxulo_runtime.a" \
  target/release/xupm build path/to/probe.xulo --release

# Repository state.
git submodule status
git ls-tree HEAD spec std-spec
```

Last recorded verification (2026-11-15):

| Check | Result |
|-------|--------|
| `cargo test --workspace` | 2019 passed, 0 failed |
| Examples (CI job) | Configured to run every `examples/*.xulo` |
| AOT smoke tests | `hello` on macOS/Windows; `structs`, `trait_shapes`, `generics`, `modules` probed locally |
| AOT differential probes | `collections`, `optional_chaining`, `Array.len` currently diverge |

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
