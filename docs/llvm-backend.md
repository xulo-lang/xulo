# Xulo LLVM Backend Integration Design

- **Status**: Accepted
- **Date**: 2026-11-12
- **Audience**: language implementers, toolchain maintainers
- **Scope**: the native AOT backend of `xulo-compiler` / `xupm`

This is an implementation design document, not a language specification. Language
semantics are defined by `spec/`; this document only covers how to compile the
existing `IrModule` into an LLVM backend artifact and how to distribute it.

## 1. Background

Current execution paths (the interpreter and AOT are independent paths; the
interpreter does not go through `IrModule`):

```
.xulo source
  → xulo-lexer / xulo-parser       (pest)
  → xulo-semantic                  (checks + dispatch/builtin rewriting)
       ├─ xulo-runtime interpreter (default path for `xulo run`, bypasses IrModule)
       └─ xulo-compiler::irgen
            → xulo-core::ir::IrModule
                 ├─ xulo-compiler::codegen  → Cranelift JIT (tests/in-process)
                 └─ xulo-compiler::aot      → .o → cc + libxulo_runtime.a
```

Goal: add an **LLVM AOT backend** so that `xupm build` produces optimized native
executables.

`IrModule` is already low-level enough (local variables, labels,
`I64/F64/Bool/Pointer/Void`, runtime calls, FFI calls, closures), so integrating
LLVM is "adding a backend", not rewriting the front end or the runtime.

## 2. Design Constraints

1. **Ordinary application developers install only the xulo compiler**, mirroring
   Rust: users install `rustc/cargo`, LLVM is already inside the compiler
   binary, and users do not install clang/LLVM.
2. **Users do not choose a backend**: `xupm build` has no `--backend`; there is
   exactly one product path.
3. **LLVM is AOT-only**: the interpreter remains the default fast execution path,
   and the Cranelift JIT continues to serve tests and differential comparison.
4. Do not modify `crates/xulo-codegen` (JS generation is retired; see
   `AGENTS.md`).
5. A default `cargo test` without LLVM enabled must not depend on an LLVM
   toolchain.

## 3. User-Visible Behavior

Today `xupm build` has only three arguments — `file / -p/--package /
--workspace` — uses Cranelift AOT, and depends on
`target/release/libxulo_runtime.a` inside the source tree. The behavior below
describes the target state; `--release` is a new interface added in M2.

```bash
xupm build            # built-in LLVM, debug profile (low optimization)
xupm build --release  # built-in LLVM, release profile (high optimization)
```

- `--release` controls only the optimization level, not the backend. This flag
  does not exist yet; M2 introduces it.
- `--backend` is not exposed; Cranelift is kept only as an internal
  implementation detail for language developers.
- Builds still require a system linker (`cc`) and the runtime static library, as
  today; this is the same level of dependency as Rust on Linux (linker + CRT).
  See 7.4 and 11 for cross-platform linking.

## 4. Why Embed LLVM Instead of Invoking clang

The early alternative was "emit `.ll` text + external `clang`". That approach is
unacceptable for ordinary developers: it forces them to install clang,
violating constraint 1.

The production path follows Rust's model:

- the `xulo`/`xupm` binary **statically links LLVM**;
- in-process `inkwell` / `llvm-sys` compiles `IrModule` into an object file
  (`.o`);
- the system linker links the `.o` with `libxulo_runtime.a` into an executable.

Text `.ll` output is kept only as a debugging aid for language developers
(see 7.6) and does not participate in product builds.

## 5. Roles and Dependency Layers

| Role | Must install | Touches LLVM? |
|------|--------------|---------------|
| Ordinary application developer | `xulo`/`xupm` binary + system linker | No; LLVM is inside xulo |
| Language developer (hacking on xulo/backend) | xulo sources + LLVM dev libraries (`llvm-config`) + clang (tests/debugging) | Yes |

Cargo feature plan:

- `xulo-compiler/llvm`: optional dependency on `inkwell` (or `llvm-sys`),
  enabling LLVM object-file generation.
- `xupm/llvm`: forwards to `xulo-compiler/llvm`. Distribution artifacts must
  enable it.

Without the `llvm` feature, `xupm build` falls back to Cranelift (for local
language development only) and prints "LLVM not enabled". **Distribution builds
must fail to build when the `llvm` feature is missing**, so a binary that
silently uses the other backend is never shipped. Also recommended: have
`xupm --version` or the build log report the backend actually used, to aid
troubleshooting.

## 6. Code Organization

Suggested additions:

```
crates/xulo-compiler/src/llvm/
├── mod.rs        # lowering from IrModule to an LLVM Module (emit)
├── object.rs     # LLVM object-file output (called by the shared build pipeline)
└── abi.rs        # IrAbiParam / RUNTIME_FNS → LLVM type mapping
```

Reuse:

- `xulo-compiler::module::{load, analyze, apply_trait_dispatch, apply_std_calls}`
  - `load(entry: &Path) -> Result<LoadedModules, XuloError>`
  - `analyze(&mut LoadedModules, check_entry: bool) -> Result<Vec<XuloError>, XuloError>`
  - `apply_trait_dispatch(&mut LoadedModules)` / `apply_std_calls(&mut LoadedModules)`
- `xulo-compiler::compile_modules_to_ir(&LoadedModules) -> Result<IrModule, XuloError>`
- `xulo-compiler::aot::link_executable(&Path, &Path)` (the only AOT step that can
  be reused directly)
- `xulo-compiler::backend::RUNTIME_FNS` (the single source of truth for runtime
  functions)
- the `staticlib` artifact of `xulo-runtime`

You **cannot** reuse `aot::write_object_file` directly: it takes a
`cranelift_object::ObjectProduct` (`aot.rs:35`), which is unrelated to the LLVM
artifact type. Factor the write-to-disk logic into a backend-agnostic form (e.g.
`write_object_bytes(&[u8], path)`), or let each backend write its own artifact
(LLVM's `TargetMachine` can write files directly).

Responsibility boundaries:

- `irgen`: source → `IrModule` (unchanged)
- `codegen.rs`: `IrModule` → Cranelift (kept)
- `llvm/mod.rs`: `IrModule` → LLVM IR (new)
- `llvm/object.rs`: LLVM object-file output (new)

## 7. Backend Implementation

### 7.1 Type Mapping

User-function ABIs must match the existing Cranelift backend:

| Source | LLVM type | Notes |
|--------|-----------|-------|
| IR value `I64` / `Bool` / `Pointer` | `i64` | IR values always occupy an i64 slot (`codegen.rs:338`) |
| IR value `F64` | `double` | floating-point locals/constants |
| `Void` | `void` | function return |
| user function params/returns | always `i64` | matches `create_signature` |
| closure entry | `(i64 × 9) -> i64` | `env` + `a0..a7`, padded with 0 |
| FFI `IrAbiParam::Bool` | `i1` (C `_Bool`) or `i8` (matching Cranelift) | Cranelift uses `I8` (`codegen.rs:1205`); returns/extensions must be verified against it |

Runtime function signatures come from `RUNTIME_FNS`: `i64` maps to `i64`, `F64`
maps to `double`. FFI imports map through `IrAbiParam`
(`Bool/S8/S16/S32/S64/U8..U64/F32/F64`).

Distinguish two layers of `Bool`:
- **IR value layer**: `IrValue::Bool` is always `iconst i64`;
- **FFI boundary layer**: only `IrAbiParam::Bool` narrows to one byte / `i1`.

Do not conflate the two (see 8.4).

### 7.2 Function-Body Lowering

Mirror `codegen.rs` instruction by instruction:

- **Locals**: create an `alloca` (`i64` or `double`) in the entry block for each
  local and use `load`/`store` for mutable bindings; with optimization enabled,
  LLVM's mem2reg converts them to SSA.
- **Parameters**: follow Cranelift and read every parameter as an `i64`.
- **Labels**: two-pass scan — create a basic block for each `Label`, then fill in
  `br`; terminate empty blocks with `unreachable` (matching `codegen.rs`'s
  "terminate empty blocks" behavior).
- **Constants**: `Bool/I64/Null` → `i64`; `F64` → `double`, also calling
  `xulo_register_float(bitcast)` to preserve runtime auto-detection; strings →
  module-level `private unnamed_addr constant [n x i8]`, then `getelementptr`
  and `ptrtoint` to `i64`.
- **String registration**: call `xulo_register_string` for every interned string
  in the entry function, matching Cranelift.
- **Arithmetic/comparison/logic**: integer `add/sub/mul` and `icmp`; floating
  `fadd/fsub/fmul/fdiv` and `fcmp`, with operand-type promotion (matching
  `numeric_binop` / `promote_pair` / `fit_numeric_slot`).
- **Division/remainder**: integers use `sdiv`/`srem`; floats use `fdiv` plus the
  `frem` equivalent. See 8.1 (division) and 8.2 (float remainder) for UB and
  current-behavior differences.
- **Shifts**: keep using the `xulo_shift` runtime function (matching
  `codegen.rs`).
- **Runtime calls**: generate `declare` from `RUNTIME_FNS`; `call` uses the C
  ABI.
- **User-function calls**: emit a direct `call`; `function_addr` uses
  `ptrtoint ptr @fn to i64`.
- **Closures**: `MakeClosure` → `xulo_alloc_closure`; `CallIndirect` →
  `xulo_check_callable` to obtain the function pointer, then an indirect call
  through the `(i64×9)->i64` signature.
- **Objects/arrays/enums/regex/std**: lower everything to the corresponding
  runtime calls; the backend does not reimplement semantics.
- **FFI**: `extern "C"` symbols are declared directly; `ExternCall` follows the
  narrow-type truncation/extension rules.

### 7.3 Build Pipeline (Refactor the Existing Flow; Do Not Add a Parallel One)

`build_executable` at `crates/xulo-compiler/src/aot.rs:93` already implements
load → analyze → dispatch/builtin rewriting → IR → object file → link, and both
`xupm` and the tests call it. Do **not** add a second
`llvm/aot.rs::build_executable`; two pipelines would drift.

Refactor it into "shared front half + replaceable IR→object-file step":

```rust
enum AotBackend { Cranelift, Llvm }

pub fn build_executable(entry: &Path, output: &Path) -> Result<BuildOutput, XuloError> {
    build_executable_with(entry, output, AotBackend::Cranelift)
}

pub fn build_executable_with(
    entry: &Path,
    output: &Path,
    backend: AotBackend,
) -> Result<BuildOutput, XuloError> {
    let mut loaded = module::load(entry)?;
    let warnings = module::analyze(&mut loaded, true)?;
    module::apply_trait_dispatch(&mut loaded);
    module::apply_std_calls(&mut loaded);
    let ir = compile_modules_to_ir(&loaded)?;

    let obj_path = output.with_extension("o");
    emit_object(backend, &ir, &obj_path)?;   // the only branch point
    let link_result = link_executable(&obj_path, output);
    let _ = std::fs::remove_file(&obj_path); // always clean up, matching aot.rs:114
    link_result?;
    Ok(BuildOutput { executable: output.to_path_buf(), warnings })
}
```

- `emit_object(Cranelift, ..)`: existing `AotCodeGen` → `ObjectProduct` →
  write to disk.
- `emit_object(Llvm, ..)`: `llvm::emit(&ir)` → `TargetMachine` → write `.o`.
- Keep `build_executable` as the Cranelift compatibility entry point so tests do
  not need to change.

### 7.4 Object Files and Linking

- The object file is emitted by `TargetMachine` using the host/`target_lexicon`
  triple.
- Linking reuses `aot::link_executable`, but that function is currently
  Linux/GNU-specific:

```
cc -no-pie -o <exe> <obj> libxulo_runtime.a \
   -lpthread -lm -ldl \
   -Wl,--gc-sections -Wl,--strip-debug
```

Cross-platform support must be handled separately (start with Linux; other
platforms are separate work items):

| Platform | Linker | Notes |
|----------|--------|-------|
| Linux | `cc` (GNU/Clang) | current flags; `-no-pie`, `--gc-sections` |
| macOS | `cc` (Clang) | whether `-no-pie` works on ld64 must be measured; replace `--gc-sections` with `-Wl,-dead_strip` (to be verified); static/system libraries differ |
| Windows | `link.exe` (MSVC) or `lld-link` | static-library suffix is `.lib`; requires the MSVC runtime/CRT |

Optimization levels:

- debug: LLVM `OptLevel::None`
- release: LLVM `OptLevel::Default`/`Aggressive`, with optional strip before
  linking

### 7.5 Distributing and Locating the Runtime Static Library

`aot.rs:126 runtime_library_path()` currently hardcodes the source-tree path
`target/release/libxulo_runtime.a` and prints "run cargo build first". That
works for source-tree development but **violates constraint 1**: an installed
`xupm` cannot find that file.

This must be solved together with distribution:

1. **Ship it in the installer**: install the runtime static library to
   `<prefix>/lib/xulo/` (or a `lib/` next to the `xupm` executable) and locate
   it at runtime relative to the executable/install prefix. Platform file names
   differ (`libxulo_runtime.a` / `xulo_runtime.lib`).
2. **Environment override**: add `XULO_RUNTIME_LIB` so development/tests can
   point at a path.
3. **Embedding** (optional): `include_bytes!` the runtime static library when
   building `xupm`, and write it to a temporary directory before linking. This
   avoids install-layout issues but increases `xupm` size and complicates cross
   builds.

Suggested lookup order: `XULO_RUNTIME_LIB` → next to the executable/install
prefix → source-tree `target/release` (development only). This must be resolved
in M0/M2; otherwise "install only xulo" does not hold.

### 7.6 Debug Output (Language Developers Only)

Provide an environment variable (e.g. `XULO_DUMP_LLVM=1`) that writes
`Module::print_to_string()`'s `.ll` into the build directory, for troubleshooting
the lowering. This path does not invoke external clang and does not affect normal
execution. Implemented in M1.

## 8. Semantic Consistency Risks

Both backends must behave identically. The following need dedicated verification.

### 8.1 Integer Division/Remainder: Define the Target Semantics First

**Current behavior**: `Div` emits Cranelift `sdiv` (`codegen.rs:386` /
`codegen.rs:1138`), and the integer branch of `Mod` emits `srem`
(`codegen.rs:394` / `codegen.rs:408`). On x64, Cranelift inserts explicit
divide-by-zero/overflow checks for both instructions (`cranelift-codegen`'s
`nonzero_sdiv_divisor` and `trap_if IntegerDivisionByZero/IntegerOverflow`); the
trap lowers to `ud2`, so with no trap handler installed the process receives
**SIGILL** rather than a Xulo diagnostic.

So the current behavior is "**a hardware trap (`ud2`, crash) after compile-time
checks**", not a language-level runtime error. The document must follow this, not
describe it loosely as "Xulo's runtime failure behavior".

**LLVM side**: `sdiv`/`srem` are UB on division by zero and `INT_MIN / -1`, so
`-O2` cannot be relied on. The target semantics must be decided before M3:

- (a) **Match current behavior**: insert explicit checks + `trap`/`abort`,
  preserving the crash behavior;
- (b) **Turn it into a language-level error**: use `panic`/`Result`. This is a
  behavior change and requires a separate decision plus updates to the
  interpreter, Cranelift, docs, and tests.

### 8.2 Float Remainder: Approximate Formula vs Exact `frem`

The `Mod` branch in `codegen.rs` (`codegen.rs:394`) currently computes float
remainder as `a - trunc(a/b) * b` (Cranelift's `trunc` is the round-toward-zero
floating instruction), and the comment calls it the interpreter's fmod semantics.
That formula is **not guaranteed to be numerically equivalent** to real
`fmod(a, b)`: `a / b` rounds first, and the error enters the result when the
quotient is large. LLVM provides an exact `frem`, so this must be decided before
M3 (together with 8.1):

- (a) **Use LLVM `frem`**: more accurate, but it will differ from
  Cranelift/interpreter on extreme values; record the difference with
  differential tests and decide whether to fix the other backends too;
- (b) **Replicate the current formula**: bit-for-bit alignment with Cranelift,
  while keeping the known precision loss.

Until this is decided, do not describe the LLVM side as an "equivalent
implementation".

### 8.3 Shift-Amount Overflow

The current path already routes everything through `xulo_shift`; the LLVM side
must not emit raw `shl/shr`.

### 8.4 `_Bool` / Narrow-Type ABI

- IR value layer: `Bool` is `i64` (`codegen.rs:338`);
- FFI layer: `IrAbiParam::Bool` maps to `I8` in Cranelift (`codegen.rs:1205`).
  The AOT narrow-type extension rules are defined by the `ExternCall`
  conversions in `codegen.rs`: `coerce_extern_arg` (argument narrowing/numeric
  conversion, `codegen.rs:1217`) and `coerce_extern_ret` (return-value
  `sextend`/`uextend`/`fpromote`, `codegen.rs:1262`). The LLVM side may use `i1`
  (C `_Bool`) or `i8` (matching Cranelift); the argument bits agree on x86-64,
  but argument/return-value/sign- and zero-extension for narrow integers must
  match the AOT code above. Note that `crates/xulo-runtime/build.rs` generates
  the **interpreter's** FFI trampolines (emitted for `interpreter::ffi`; see the
  comment at the top of that file); it is only an interpreter-side reference, not
  the basis for AOT.

### 8.5 Float Registration Points

`xulo_register_float` is called in more places than compiler constants:

- `codegen.rs:344`: `F64` constants;
- `xulo-runtime/src/runtime/heap.rs:120`: non-integer elements in range lists;
- `xulo-runtime/src/runtime/json.rs:107`: JSON numeric payloads.

The latter two happen inside runtime functions, and both backends share the same
runtime, so they are not backend divergence. However, **the set of
compiler-generated constant registration points must match Cranelift exactly**;
"register every F64 constant/default value/constant-folded result" should be a
differential-test item, otherwise `print`'s tag detection will differ.

### 8.6 Other

- **String interning and null pointers**: `Null` uses `i64 0`; string constants
  must be NUL-terminated.
- **Local variable types**: `F64` locals must use `double` slots and parameters
  must use `i64` slots, to avoid implicit bit-pattern misreads.

Use **differential tests** as the safety net: run the same source through both
the Cranelift JIT and the LLVM executable, comparing stdout, exit codes, and
runtime error text.

## 9. Testing and CI

- **Emitter golden tests**: assert function signatures, string constants,
  closure entries, control flow, and extern declarations in the emitted `.ll`.
  Because the emitter depends on the LLVM API (inkwell), these tests must be
  compiled under the `llvm` feature and belong to the LLVM CI job, not the
  default job.
- **Differential tests**: reuse the corpus in
  `crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs`; the LLVM side
  requires the `llvm` feature.
- **Switch semantics**: `llvm` is a **compile-time feature** (whether LLVM code
  is compiled in); `XULO_LLVM=1` is a **runtime test switch** (force the
  differential tests to run when the feature is enabled; skipped by default to
  save time), implemented in M2. They are not alternatives; when the feature is
  disabled, the environment variable has no effect.
- **End-to-end**: after `xupm build --release`, run the artifact and verify its
  output.
- **CI**: install a matching LLVM development library and add a build/test job
  with `--features llvm`; the default job stays free of LLVM dependencies.

## 10. Milestones

**M0: Feasibility Spike (First)**
- statically link `inkwell`/`llvm-sys` into `xulo-compiler`;
- compile a minimal `IrModule` (hello) to `.o` in-process;
- link and run it successfully via `link_executable` (Linux first);
- confirm the static-linking approach, LLVM version, binary size, and build
  command;
- decide the runtime static-library install/lookup scheme (7.5).

**M1: Complete the Emitter**
- cover every `IrModule` instruction in `llvm/mod.rs`;
- add golden tests (under the `llvm` feature);
- implement `XULO_DUMP_LLVM` (7.6);
- compare instruction by instruction against Cranelift.

**M2: Wire Up `xupm build`**
- refactor the existing `aot::build_executable` per 7.3, adding the backend enum
  and `emit_object`;
- add the `xupm/llvm` feature; `xupm build [--release]` uses LLVM (`--release`
  is new);
- without the feature, fall back to Cranelift and warn; distribution builds fail
  when the feature is missing;
- implement the `XULO_LLVM` test switch (section 9);
- differential and end-to-end tests.

**M3: Semantic Alignment and Release**
- decide and implement integer-division and float-remainder semantics per
  8.1/8.2; fix ABI, float-registration, and other differences;
- cross-platform linking (macOS/Windows) is separate work;
- CI covers the LLVM job;
- distribution builds enable `llvm` by default; update docs (README, install
  instructions).

## 11. Distribution and Size

- Prefer **statically linking LLVM**: users install only xulo, closest to Rust.
  The cost is a significantly larger binary and the need for a matching LLVM on
  release builders and CI.
- Alternative: ship a `libLLVM.so` alongside xulo and set rpath; users still
  "install only xulo" but must manage the dynamic library and search path.
- The runtime static library must be installed with the package and located per
  7.5; it must not rely on a source-tree path.
- Linker dependencies differ by platform (7.4); "install only xulo" really means
  "install xulo + the platform-standard linker/CRT", the same as Rust.
- Cross compilation is harder with a statically linked LLVM; evaluate the target
  platform range in M0/M3.

## 12. Open Questions

- `inkwell` or `llvm-sys` directly (the former is safer, the latter more
  controllable).
- LLVM version pinning and the release image must be decided together: the
  `llvm-sys` major, the CI image, and release builders must use the same LLVM
  version.
- Whether to enable LTO / strip / debug info in the release profile.
- Whether to eventually use a bundled LLD to drop the system linker (not doing
  this now; it matches Rust's default behavior).
- Scope of the differential-test corpus: compiler tests only, or all of
  `examples/`.
