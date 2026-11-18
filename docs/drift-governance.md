# Xulo Drift Governance

- **Status**: Living document
- **Date**: 2026-11-18
- **Audience**: compiler and runtime maintainers, stage 1 bootstrap team
- **Normative sources**: `spec@48f49b7` and `std-spec@a2e4cb7`, as pinned by this repository
- **Companions**: [`stage1-readiness.md`](stage1-readiness.md), [`tasks.md`](tasks.md), [`../xuloc/BOOTSTRAP.md`](../xuloc/BOOTSTRAP.md)
- **Implementation status**: the invariants and the ledger below are normative.
  The harness commands in §10, the CI tiers in §9, and the differential gate in §5.4 are being introduced;
  until they land, `crates/xupm/tests/stage1_build.rs` provides partial
  evidence for I1–I4.

This document defines how semantic drift between Xulo execution engines is
detected, bounded, and eventually removed. It is the authoritative description
of the stage-comparison gate: if a test and this document disagree, this
document wins and the test is wrong.

## 1. Purpose

Xulo currently has three execution engines and two independent semantic
implementations:

| Engine | Semantic implementation | Coverage |
|---|---|---|
| Interpreter (`xulo run`) | Rust (`crates/xulo-runtime`) | the whole language |
| AOT (`xupm build`) | Rust (the same `xulo_*` ABI) | the whole language |
| stage 1 C backend (`xuloc build`) | `xuloc/runtime.c` (reimplemented) | the bootstrap subset |

Drift is the property that the same program produces different observable
behavior on different engines. Because the third row is an independent
reimplementation, drift is not an accident: it is produced continuously by
(a) features that land only in stage 0, (b) fixes applied to only one side, and
(c) a comparison matrix that is smaller than the feature surface.

The goal is not "no two implementations". That decision is tracked separately
(see §12). The goal is that **every divergence is either impossible or
observable**, and that no divergence can be introduced silently.

## 2. Vocabulary

- **Engine**: a binary that executes Xulo source (interpreter, AOT output,
  stage 1's C backend output, stage 2).
- **Stage**: a compiler generation. Stage 0 is the Rust toolchain; stage 1 is
  the compiler written in Xulo; stage 2 is stage 1 compiled by stage 1;
  stage 3 is stage 2 compiled by stage 2.
- **Oracle**: the authority that decides what a program *should* do. This
  repository uses two: the pinned specification (conformance) and stage 0
  (behavioral reference).
- **Drift**: a difference in `exit code`, `stdout`, `stderr`, or emitted C
  between two engines for the same input and command.
- **Observation**: the tuple above, normalized per §6.

## 3. The stage-comparison invariant

Stage 1 is the compiler source `xuloc/*.xulo` compiled by stage 0's AOT backend
(LLVM or Cranelift). Stage 2 is the *same source* compiled by stage 1's own C
backend. Comparing them therefore compares **two backends on one large, real
workload**, not two versions of the compiler.

The following invariants are normative.

| ID | Invariant | What it validates |
|---|---|---|
| **I1** | `emit_c(stage1, P) == emit_c(stage2, P)` byte for byte | backend determinism and a true fixpoint |
| **I2** | `run(stage1, cmd, P) ≡ run(stage2, cmd, P)` for `exit`, `stdout`, `stderr` | front end, checker, diagnostics, `build` status |
| **I3** | `run(build(stage1, P)) ≡ run(build(stage2, P))` | end-to-end runtime behavior of generated programs |
| **I4** | `emit_c(stage2, P) == emit_c(stage3, P)` | idempotence after the first fixpoint |

I1 is the primary invariant. I3 adds link-time and startup differences that I1
cannot see.

## 4. What stage comparison proves, and what it does not

**Proves:**

- stage 1's C backend compiles the compiler itself, on this large workload;
- the emitted C is deterministic (no hash-order, pointer, time, or path leaks);
- there is no hidden stage-dependent behavior;
- differences between the Cranelift/LLVM backend and the C backend are caught
  on a large program.

**Does not prove:**

- conformance to the specification;
- correctness of behavior shared by both stages.

A bug in the shared front end, or a semantic gap in both backends, produces
consistent but wrong output on both sides. Stage comparison is a **differential
invariant, not an oracle.** The external oracle (§5.2) remains mandatory.

Stage comparison has one further property that makes it the primary gate: it
needs no stored golden files. There is nothing to "accept" or refresh, so it
cannot rot.

## 5. Engine matrix

### 5.1 Stage comparison (primary)

| Input | `tokens` | `parse` | `check` | `build` | build output run |
|---|---|---|---|---|---|
| `xuloc/*.xulo` | I2 | I2 | I2 | I1, I2 | I3 |
| `xuloc/corpus/accept/*` | I2 | I2 | I2 | I1, I2 | I3 |
| `xuloc/corpus/reject/*` | I2 | I2 | I2 | expected rejection | — |
| `xuloc/corpus/semantics/accept/*` | — | — | — | I1, I2 | I3 |
| `xuloc/corpus/semantics/fail/*` | — | — | — | I2 (exit, stderr) | — |

`build` compares the generated C bytes as the primary object. The binary is
never compared byte for byte: link timestamps and embedded paths make that
meaningless.

### 5.2 External oracle (secondary)

The pinned specification and stage 0 answer "is this correct?", which stage
comparison cannot. This layer runs the pinned `// ok` examples and the semantic
corpus through stage 0, and stage 1 where the case is inside the bootstrap
subset. Where the specification and stage 0 disagree, the specification wins
(§7).

### 5.3 Backend cross-check (stage 0)

`xupm build --backend llvm|cranelift` builds two stage 1 compilers from the same
source. Comparing them applies I1–I3 to the stage 0 backend axis. This is cheap
(no C compiler) but requires a binary built with the `llvm` feature; it runs in
the LLVM CI job only.

An explicit `--backend llvm` on a binary without the `llvm` feature **must
fail**. It must not warn and fall back to Cranelift, because that would compare
Cranelift with itself and silently pass.

### 5.4 Stage 0 / stage 1 differential gate

Every case in the bootstrap subset (the §5.1 inputs) is run through stage 0 and
through stage 1's C backend, and the §6 observations are compared. Each
observed difference must match a ledger row (§8) by ID and by case; an
unmatched difference fails the gate. In a full run, a ledger row that matches no
difference is stale and must be removed or reclassified.

This gate is what makes §1's "no silent divergence" hold for stage 0 against
stage 1. Without it the ledger only records the differences someone happened to
notice.

## 6. Normalization rules

Two observations are compared only after all of the following hold:

1. **Same input path and working directory.** Diagnostics embed the path as
   passed on the command line.
2. **Same environment.** `XULO_CC`, `XULO_STAGE1_RUNTIME`, and every other
   variable are identical on both sides.
3. **`build` reuses the same output paths.** Run the engines sequentially with
   one shared `XULO_STAGE1_C` / `XULO_STAGE1_OUT`; `build` prints the artifact
   path, so distinct paths would create a difference that is not drift.
4. **Line endings are normalized to `\n`** before comparison.
5. **Emitted C is compared raw**; it must already be path independent.
6. **Binaries are not compared**; only their behavior.

## 7. Failure policy

- A stage1/stage2 difference is **always a bug**. There is no baseline, waiver,
  or "known" list for I1–I4. The build fails.
- A stage0/stage1 difference that has no ledger row fails the gate (§5.4).
  A listed difference is either fixed (and its row removed) or kept with status
  `accepted` (§8).
- Where the specification and stage 0 disagree, the specification is
  authoritative and the stage 0 behavior is a bug.
- A missing runtime, compiler, or linker is a code generation environment
  error and must produce a coded diagnostic, never a crash. It is `E0005`,
  which BOOTSTRAP defines as code generation and its external toolchain. A
  file I/O failure is `E0004`.
- A gate step that cannot run fails; it never passes by skipping. The
  conformance CI step sets `XULO_STAGE1_REQUIRED`; if `XULO_STAGE1_BUILD` is
  missing while it is set, `crates/xupm/tests/stage1_build.rs` panics. Local
  runs without the variables still skip.

## 8. Known deviations ledger

Each entry names its evidence, the cases that expose it (§5.4), and the change
that removes it. New entries require a row here in the same change. This ledger
covers stage0-vs-stage1 only; it never covers stage comparison.

Status values:

- `open`: a fix is planned; the row is removed when the fix lands.
- `accepted`: kept on purpose for a stated scope (for example, the bootstrap
  subset). The scope must be written in the row, and the row is re-reviewed at
  each gate review (§13).

| ID | Deviation | Evidence | Removal | Status |
|---|---|---|---|---|
| D-01 | Integral `+`/`-`/`*` wrap silently in the C backend; Rust traps with `panic: integer overflow` | `xuloc/runtime.c` `xr_wrap_*` vs `crates/xulo-runtime/src/runtime/scalar.rs::xulo_int_add` | C backend traps on overflow, as Rust does | open |
| D-02 | Float printing uses `%.15g`/`%.17g`; Rust uses shortest round-trip | `xr_format_float` vs `xulo_std::format::format_number` | C backend uses the same shortest round-trip rule as `xulo_std` | open |
| D-03 | Runtime failure text differs (`division by zero` vs `panic: division by zero`) | `xr_fail` vs `trap` | `xr_fail` prints the same `panic: ` prefix and text as `trap` | open |
| D-04 | `u64` has no tag in the C backend and is treated as `int` | `xuloc/runtime.c` `XT_*` enum | Add a `u64` tag to the C backend, or reject `u64` with `E0005` in the bootstrap subset | open |
| D-05 | R8 value semantics are not reproduced: structs are reference-shaped | `xuloc/BOOTSTRAP.md` item 20 | Implement value semantics, or reject the dependent constructs in the bootstrap subset. Scope: bootstrap subset only | accepted |
| D-06 | A failing C compiler makes the native stage 1 abort instead of reporting `E0005` | AOT mis-tagging of `exec` output concatenation | Tag-propagation fix (§11); remove the row when it lands and its test passes | open |

## 9. CI tiers

| Tier | Where | Contents | Budget |
|---|---|---|---|
| Fast | `conformance` job, PR | cached stage 2; I1/I2 on all sources and the corpus; failure paths | < 60 s cached, < 4 min cold |
| Full | `main` / nightly | stage 2 rebuilt from scratch; full matrix including I4 and the semantics corpus | < 10 min |
| LLVM cross-check | LLVM job | `--backend llvm` vs `--backend cranelift`, I1–I3 on the same matrix | < 10 min |

The stage 2 cache key is the hash of `xuloc/**` plus the stage 1 binary hash and
the toolchain identity. A cache miss falls back to the full build; it must never
produce a vacuous pass.

## 10. Running locally

```bash
# Fast: needs a C compiler and the release runtime library.
cargo build -p xulo-runtime --release
XULO_STAGE1_BUILD=1 cargo test -p xupm --test stage1_build -- --nocapture

# Full: rebuild stage 2 from scratch and compare every input.
XULO_STAGE1_BUILD=1 XULO_STAGE1_FULL=1 cargo test -p xupm --test stage1_build -- --nocapture

# Stage 0 backend axis (requires a binary built with `--features llvm`).
cargo test -p xupm --test stage1_build backend_cross_check -- --nocapture
```

## 11. Change procedure

- Adding a language feature to stage 0 requires either stage 1 support or an
  explicit `E0005` rejection with a test (§5.1, "expected rejection").
- Adding a test case means adding it to the stage comparison matrix *and*, if it
  touches the bootstrap subset, to the semantic corpus.
- Fixing a ledger entry removes the row in the same change.
- The tag-propagation fix that closes D-06 must land before the backend
  cross-check is made blocking, because the cross-check will otherwise report
  the same defect twice.

## 12. Runtime unification

Retiring the duplicate implementation — either by retargeting the C backend to
the Rust runtime or by letting `xupm` own the C toolchain — is out of scope for
this document. The two candidate routes, their blockers, and their costs are
tracked in `docs/tasks.md`. Whatever route is chosen, this document's invariants
stay in force during and after the migration: every step must keep I1–I4 green.

## 13. Maintenance

- Update the ledger in the same change that fixes or introduces a deviation.
- Record the stage 1 commit, the stage 0 commit, and the pinned spec commits at
  each gate review, in the "门禁评审记录" section of
  [`stage1-readiness.md`](stage1-readiness.md).
- When the matrix grows, update §5 in the same change.
- Keep this document in sync with `stage1-readiness.md` §3 status vocabulary.
