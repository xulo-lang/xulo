# Stage 1 bootstrap notes

Stage 1 is a self-hosted compiler for the bootstrap subset, written in Xulo:
a lexer, parser, first semantic checker, and a C backend that turns the generic
AST into portable C. Stage 0 (the Rust toolchain) runs or compiles it; the
native stage 1 binary then compiles itself into stage 2. Gates 1–3 prove the
front end; WP6 closes the self-host loop (Gates 5–6).

## Layout

```
xuloc/
├── xupm.toml          # xupm package manifest (entry `src/main.xulo`)
├── BOOTSTRAP.md       # this file: design notes and bootstrap workarounds
├── src/               # the compiler, written in Xulo
│   ├── main.xulo      #   CLI: tokens / parse / check / build / scan
│   ├── ast.xulo       #   generic Node shape and tree renderer
│   ├── lexer.xulo     #   UTF-8 scanner
│   ├── parser.xulo    #   recursive-descent parser
│   ├── checker.xulo   #   first semantic pass
│   ├── backend.xulo   #   portable-C code generator
│   └── tests.xulo     #   stage 1's own #[test] functions
├── runtime/
│   └── runtime.c      # dynamic runtime linked into generated programs
└── corpus/            # Gate 3 accept/reject fixtures
    ├── accept/
    └── reject/
```

Sources live under `src/` because `mod ast;` resolves next to the file that
declares it, so `main.xulo` and every module it names stay together. `tests.xulo`
also lives in `src/` for the same reason. The C runtime is not Xulo source, and
the corpus is not compiled by the package, so they stay outside `src/`.

| File | Role |
|------|------|
| `xuloc/xupm.toml` | `xupm` package manifest: name `xuloc`, entry `src/main.xulo`, artifact `xuloc/.xupm/build/xuloc` |
| `xuloc/src/ast.xulo` | One generic `Node` shape plus the stable tree renderer |
| `xuloc/src/lexer.xulo` | UTF-8 scalar scanner: tokens, spans, line/column, comments, literals, operators |
| `xuloc/src/parser.xulo` | Recursive-descent parser: headers, declarations, statements, expressions, patterns, types |
| `xuloc/src/checker.xulo` | Name and type checking for the Gate 3 corpus: functions, bindings, control flow, lists, optionals, calls |
| `xuloc/src/backend.xulo` | Code generator: generic AST to portable C, declaration collection, expressions/statements, method/static/path calls, patterns |
| `xuloc/runtime/runtime.c` | Dynamic bootstrap runtime linked by generated programs: tagged `XVal` values, structs/enums, method registry, `std::fs`/`std::io`/`std::os`/`std::process`/`std::path`/`std::hint`/`std::math`/`std::json` |
| `xuloc/corpus/accept/` | Files stage 1 and stage 0 must both accept; `.expected` files hold their compiled output |
| `xuloc/corpus/reject/` | Files both must reject, with a coded semantic diagnostic |
| `xuloc/src/main.xulo` | CLI: `tokens`, `parse`, `check`, and `build` modes, diagnostics, exit status |
| `xuloc/src/tests.xulo` | Stage 1's own `#[test]` functions, including self-parse and corpus checks |

`crates/xulo-cli/tests/stage1.rs` drives the same files through the stage 0
binary, and `crates/xupm/tests/cli.rs` verifies that `xupm test` runs the
`#[test]` functions in `xuloc/src/tests.xulo`.

`ast`, `lexer`, `parser`, `checker`, and `backend` are modules, so check them
through an entry point (`xuloc/src/main.xulo` or `xuloc/src/tests.xulo`). Checking
`xuloc/src/parser.xulo` on its own cannot resolve the sibling `ast` module, by
design.

## Running

`xulo run` does not forward script arguments, so stage 1 accepts either a
compiled binary's argv or two environment variables:

```bash
# Interpreter path (Gate 1/2)
XULO_STAGE1_MODE=tokens XULO_STAGE1_FILE=examples/functions/fibonacci.xulo \
  target/debug/xulo run xuloc/src/main.xulo

XULO_STAGE1_MODE=parse XULO_STAGE1_FILE=xuloc/src/parser.xulo \
  target/debug/xulo run xuloc/src/main.xulo

XULO_STAGE1_MODE=check XULO_STAGE1_FILE=xuloc/corpus/accept/basics.xulo \
  target/debug/xulo run xuloc/src/main.xulo

# Corpus scan: one line per .xulo file, `<path>\t<lexer>\t<parser>`.
# This is what the T9 ratchet measures (docs/tasks.md WP3b).
XULO_STAGE1_MODE=scan XULO_STAGE1_DIR=examples \
  target/debug/xulo run xuloc/src/main.xulo

# Native path once a binary exists
xuloc tokens examples/functions/fibonacci.xulo
xuloc parse examples/functions/fibonacci.xulo
xuloc check xuloc/corpus/accept/basics.xulo
xuloc build xuloc/corpus/accept/basics.xulo
```

`build` generates C, invokes the host C compiler, and prints the binary path.
It writes `<dir>/.xupm/build/<stem>.c` and the executable next to the source by
default. The pieces are overridable:

| Variable | Meaning |
|----------|---------|
| `XULO_STAGE1_OUT` | Output binary path |
| `XULO_STAGE1_C` | Generated C path |
| `XULO_STAGE1_RUNTIME` | Runtime source (default `xuloc/runtime/runtime.c`, resolved from the working directory) |
| `XULO_STAGE1_DIR` | Directory scanned by `XULO_STAGE1_MODE=scan` |
| `XULO_STAGE1_SKIP` | Semicolon-separated paths the scan reports as `skipped` (inputs that exhaust the interpreter's call depth) |
| `XULO_CC` | C compiler; when unset stage 1 tries `clang`, then `cc`, then the standard LLVM install on Windows |

The generated C includes the runtime by base name and is compiled with
`-I <runtime dir>`. Windows gets `-Wl,/STACK:67108864` (the runtime and stage 1
recurse deeply); non-Windows builds add `-lm`.

The default mode is `parse`. Diagnostics use the stage 0 codes and one-line
shape `<file>:<line>:<column>: error[E…] <kind>: <message>` (`E0001` lexical,
`E0002` syntax, `E0004` I/O, `E0005` code generation and its external
toolchain, such as the C compiler and linker). Semantic failures use
`E0101` (missing name), `E0201` (type mismatch), `E0202` (argument mismatch),
`E0211` (no common type), `E0213` (not iterable), and `E0215` (not indexable).
All failures exit 1. `check` prints `no errors` on success, matching
`xulo check`.

## Code generation and the self-host loop

`xuloc/src/backend.xulo` collects declarations (functions, methods, structs, enums,
consts, imported modules), assigns C names, and emits one C translation unit.
Every Xulo value becomes an `XVal` tagged value, so the backend does not need
type information; structs and enums are field tables, and methods go through a
runtime registry. The generated `main` calls the program's `main`, if any.

`xuloc/` is an ordinary `xupm` package. `xupm` discovers `xuloc/xupm.toml`,
uses `src/main.xulo` as the entry, and names the artifact after the package
(`xuloc/.xupm/build/xuloc`), so stage 1 builds and tests also work without a
file argument.

To bootstrap:

```bash
# stage 0 builds the native stage 1 compiler
XULO_RUNTIME_LIB="$PWD/target/release/libxulo_runtime.a" \
  target/release/xupm build xuloc/src/main.xulo --release
# from inside the package the entry comes from the manifest:
#   cd xuloc && xupm build --release

# stage 1 compiles itself into stage 2 (it picks the host C compiler)
XULO_STAGE1_OUT=xuloc/.xupm/build/stage2 \
  xuloc/.xupm/build/xuloc build xuloc/src/main.xulo
```

`crates/xupm/tests/stage1_build.rs` automates the whole loop: the accept corpus,
the self-build, the stage 1/stage 2 differential (T5c), and the reproducible-C
fixpoint (B8b). It is opt-in:

```bash
XULO_STAGE1_BUILD=1 cargo test -p xupm --test stage1_build -- --nocapture
```

The runner takes about four minutes: stage 1's front end is written in the
bootstrap subset and takes roughly two minutes per compiler build. It is a
build-time gate, not a per-commit unit test; CI runs it in the `conformance`
job.

## Output formats

Tokens, one per line:

```
line:column kind start:end text
```

The tree, one node per line:

```
kind value @line:column [start,end)
```

`value` is omitted when empty. Spans are half-open and measured in Unicode
scalar offsets, matching `string` indexing in Xulo. EOF is printed with empty
text and a zero-width span.

## Bootstrap subset

The parser covers the rows stage 1 relies on: UTF-8 lexing (L1), `let` and
assignment (L4), functions and recursion (L5), control flow (L7), enums and
`match` (L8), structs and `impl` (L9), generics in declarations and types
(L10), modules and `use` (L12), optionals and `??`/`?.` types (L16), template
literals (L3/C7), and `Result`/`Error` matching (L19). It also recognises
`defer`, `async`/`await`, attributes, and trait/impl declarations well enough to
skip through them.

The parser is deliberately syntactic. `xuloc check` adds the first semantic
pass: function signatures are collected before bodies, local bindings are
scoped, calls are checked for arity and argument types, `if`/`while` conditions
must be `bool`, `for` iterables must be `List`, returns must match the declared
return type, and operators check their operand types. The fixed corpus in
`xuloc/corpus/` is the Gate 3 evidence; `crates/xulo-cli/tests/stage1.rs`
requires stage 1 and stage 0 to agree on accept/reject for every file.

The checker is not yet a full bootstrap checker. `match`, enums, structs,
traits, impls, generics, modules, closures, and std/module calls report E0003
or leave `CheckResult.complete = false` ("not supported / not modeled") rather
than being silently accepted. `build` runs the checker and stops only on a
confirmed error, so the partial analysis never rejects a program that code
generation could handle.

## Workarounds and deliberate limits

Every item below is a place where stage 1's source or parser stays inside the
currently proven subset. None of them changes the language; they are recorded
so stage 2 does not inherit them silently.

1. **No script arguments under `xulo run`.** `std::process::args()` exposes the
   host process argv, not arguments after the script path. Stage 1 therefore
   reads `XULO_STAGE1_MODE` and `XULO_STAGE1_FILE`; a compiled stage 1 binary
   uses ordinary argv. This is the reason `main.xulo` has two input paths.

2. **Inherent methods are not visible across modules.** A struct declared in
   one module exposes its `impl` methods to that module, but a second module
   that imports the type cannot call them (`E0102`). `ast.xulo` therefore
   exposes `add(node, child)` as a free function instead of `Node.add`.
   `Lexer` and `Parser` methods stay private to their own modules. Stage 1's
   source never relies on cross-module method dispatch.

3. **`not` is not the boolean prefix operator.** The language spells negation
   `!`; `not` appears only in `is not`. `lexer.xulo` uses `!` and never writes
   `not expr`.

4. **No postfix `?` propagation in the stage 1 parser.** The lexer emits `?`,
   and types accept `T?`, but expression postfix `?` is not parsed. Stage 1
   avoids `value?` and matches `Result` explicitly. This also keeps the
   ternary/postfix ambiguity out of the bootstrap parser.

5. **No call-site type arguments.** `f<int>(x)` is not consumed; stage 1 calls
   only monomorphic functions and constructors. Type arguments in type
   positions (`List<Token>`) are parsed.

6. **Closures are only `fn(...) { ... }`.** Arrow closures are not parsed.
   Stage 1 uses named module-level functions, so it never needs a closure.

7. **No template interpolation in stage 1's source.** The lexer recognises
   backtick templates and balances `${ ... }`, but stage 1's own strings use
   quotes and `+` so that the bootstrap source stays readable to the small
   parser. Nested block comments are not used either; comments do not nest.

8. **No destructuring binding in stage 1's source.** The parser skips a
   `let { ... }` or `let ( ... )` pattern, but stage 1 itself binds one name at
   a time and compares optionals to `null` or uses `??`.

9. **No struct patterns in `match`.** Variant payloads and literals are
    parsed; stage 1 matches `Result::Ok(value)`/`Result::Err(_)` and plain names
    only.

10. **Match arms carry expressions or blocks, not statements.** A `return` in
    an arm is therefore written `=> { return ... }`. This is the language rule,
    not a stage 1 extension; it is called out because it is easy to trip over
    while writing bootstrap code.

11. **String scanning is character based.** The lexer splits the source once
    with `split("")`, then indexes the resulting list, so token spans are O(1)
    per character. It does not use `string.code_at` inside the hot loop.

12. **`mut self` is the mutation tool.** `Lexer` and `Parser` keep their cursor
    and error state in struct fields and mutate them with `mut self` methods;
    recursive calls such as `self.expr()` work on the caller's binding. This is
    the proven L9/L10 pattern stage 1 relies on.

13. **Scope exit does not use `pop`/`truncate`.** `checker.xulo` records a
    scope id per binding and keeps an active-scope count. Leaving a block only
    decrements the count, so old bindings become invisible without list
    removal. This keeps the checker inside the AOT-lowered list operations.

14. **Types are canonical strings.** `checker.xulo` represents `number`,
    `string`, `bool`, `unit`, `unknown`, `null`, `List<T>`, and `T?` as strings.
    Type comparison is therefore string equality, avoiding the structural
    equality gap for tuples/lists and keeping the checker small.

15. **AOT-safe members only.** The native build (Gate 4/B8a) does not lower the
    list members `.includes` and `.join`; the lexer uses an explicit keyword
    loop and a `StringBuilder` loop for token text. The interpreter accepts
    both forms, but stage 1's own source stays inside the subset its native
    binary can execute.

16. **The backend lowers a smaller subset than stage 0.** `xuloc/src/backend.xulo`
    handles functions, structs, enums, consts, `impl` methods, modules,
    `if`/`while`/`for`/`match`, calls, fields, indexing, lists, tuples,
    object literals (and their spread), `let` object/tuple destructuring,
    struct/enum/literal/or/range/type patterns with guards, templates,
    operators, `??`, `is`, `?.`, `#[derive(Debug)]` rendering, and the
    `std::fs`/`std::io`/`std::os`/`std::process`/`std::path`/`std::hint`/
    `std::math`/`std::json` calls stage 1 uses. Module-qualified names
    (`alias::fn`, `m::CONST`) resolve by their final segment because
    declaration collection flattens modules. `Trait::method(receiver, …)`
    lowers to runtime method dispatch on the receiver's struct type.
    Closures, `Set`, `Map`/`Set` member methods and iteration, `defer`, and
    async are rejected with `E0005`; a two-part path call that is neither a
    user function, a known enum variant, a trait method, nor a known static
    method is also an `E0005` instead of silently becoming a variant
    constructor. Stage 1's own source avoids all of the rejected forms.

17. **The self-build needs the native stage 1 binary.** Running `build` under
    `xulo run` exceeds the Rust interpreter's call-depth limit (128) while
    emitting `parser.xulo`/`backend.xulo`. Build stage 1 with `xupm` and let
    that binary compile itself; the interpreted `tokens`/`parse`/`check` paths
    are still fine. This is a stage 0 limit, not a stage 1 semantic change.

18. **`and`/`or` must not carry side effects under the native path.** Stage 0's
    native builds do not reliably short-circuit them (observed on Windows and
    macOS), so stage 1 writes guards as nested `if`s when the right side could
    fail instead of `a and may_panic()`. The C backend lowers
    `and`/`or` to a ternary without a temp (so nesting short-circuits there),
    but any operand whose lowering needs a statement — a nested `??`, a call
    that materialises a temporary — is still emitted eagerly. `main.xulo`
    therefore keeps the argv guard as nested `if`s, and `literal_expr` nests
    its length check before slicing an octal prefix.

19. **The bootstrap runtime is deliberately dynamic.** `xuloc/runtime/runtime.c`
    represents every value as a tagged `XVal`, keeps structs as field tables,
    dispatches methods through a registry, and leaks memory (the compiler is a
    one-shot process). Struct values are reference-shaped, so the R8
    value-copy table is not reproduced. This keeps the first backend small; the
    Rust stage 0 runtime remains the product runtime.

## Known gaps (tracked, not hidden)

- `check` is a first semantic pass over the generic AST, not a typed tree. It
  covers the Gate 3 corpus; structs, enums, traits, impls, generics, modules,
  closures, and `match` still report E0003. `build` runs `check` first and
  refuses only a *confirmed* error: `CheckResult.complete` is `false` whenever
  the checker reached a construct, type, or call it does not model, so a
  partial analysis never rejects a program that code generation could handle.
- The C backend and the self-host loop are implemented (Gates 5–6), but the
  backend's subset is the bootstrap subset listed above. The runtime covers
  the `std` surface stage 1's own source needs plus `std::path`, `std::hint`,
  `std::os`, `std::math`, `std::json`, and `std::fs` (its `read_dir` / `walk`
  / `realpath` are POSIX-only). Object literals lower to runtime objects, so
  member access and `{ ...base, k: v }` work; `Map`/`Set` member methods,
  iteration, `std::time`, `std::fmt`, `std::regex`, and `std::net` are not
  lowered yet, so examples that use them fail with `E0005`.
- Destructuring (`let (a, b)`, `let { x, y }`), struct/enum/literal/or/range/
  type patterns with guards, and `#[derive(Debug)]`'s `Type(field: value)`
  rendering are lowered. A struct without the derive still renders as
  `Type{ field: value }`, which is stage 1's own fallback form.
- Postfix `?`, arrow closures, call-site type arguments, macros beyond
  attributes, and `spawn`/`lock` are outside the parser's accepted subset.
- The parser recovers only by stopping at the first error. That is enough for
  Gate 1/2 diagnostics; stage 1's checker work will need recovery.
- The tests read `xuloc/src/*.xulo` relative to the repository root, like the
  rest of the repository's smoke tests.

## Verification

```bash
./target/debug/xulo check xuloc/src/main.xulo
./target/debug/xulo test xuloc/src/tests.xulo
./target/debug/xupm test xuloc/src/tests.xulo
XULO_STAGE1_MODE=check XULO_STAGE1_FILE=xuloc/corpus/accept/basics.xulo \
  ./target/debug/xulo run xuloc/src/main.xulo
cargo test -p xulo-cli --test stage1
cargo test -p xupm --test cli

# Gate 4/B8a: native stage 1 matches the interpreter.
XULO_RUNTIME_LIB="$PWD/target/release/libxulo_runtime.a" \
  target/release/xupm build xuloc/src/main.xulo --release
XULO_AOT_DIFFERENTIAL=1 XULO_RUNTIME_LIB="$PWD/target/release/libxulo_runtime.a" \
  cargo test -p xupm --test aot_differential stage1_binary_matches_the_interpreter

# WP6/Gates 5–6: corpus builds, self-build, T5c, B8b (~4 minutes).
XULO_STAGE1_BUILD=1 cargo test -p xupm --test stage1_build -- --nocapture
```
