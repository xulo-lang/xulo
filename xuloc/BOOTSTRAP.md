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
│   ├── parser.xulo    #   parser state, `parse` entry, module list
│   ├── parser/        #   procedural parser modules (see below)
│   ├── checker.xulo   #   first semantic pass
│   ├── backend.xulo   #   codegen entry, collection, emitter state
│   ├── backend/       #   procedural emission modules (see below)
│   └── tests.xulo     #   stage 1's own #[test] functions
├── runtime/           # unity-built dynamic runtime for generated programs
│   ├── runtime.c      #   entry point; includes the modules below
│   ├── runtime_internal.h # shared XVal/container definitions
│   ├── runtime_value.inc  # allocation, UTF-8, constructors, containers, closures
│   ├── runtime_ops.inc    # arithmetic, comparison, `is`, nullish
│   ├── runtime_render.inc # formatting and StringBuilder
│   ├── runtime_methods.inc # member/static dispatch
│   ├── runtime_regex.inc   # std::regex
│   ├── runtime_fs.inc     # std::fs / os / process
│   ├── runtime_io.inc     # std::io readers and terminal probes
│   ├── runtime_net.inc    # std::net TCP sockets
│   ├── runtime_json.inc   # std::json
│   ├── runtime_path.inc   # std::path
│   ├── runtime_math.inc   # std::math
│   ├── runtime_fmt.inc    # std::fmt
│   ├── runtime_time.inc   # std::time
│   └── runtime_process.inc # std::process::exec
└── corpus/            # Gate 3 accept/reject fixtures
    ├── accept/
    └── reject/
```

Sources live under `src/` because `mod ast;` resolves next to the file that
declares it, so `main.xulo` and every module it names stay together. `tests.xulo`
also lives in `src/` for the same reason. A file-backed module's home directory
is its stem: `parser.xulo` declares `mod tokens;`, which resolves to
`parser/tokens.xulo`, and so on. The C runtime is not Xulo source, and the
corpus is not compiled by the package, so they stay outside `src/`.

| File | Role |
|------|------|
| `xuloc/xupm.toml` | `xupm` package manifest: name `xuloc`, entry `src/main.xulo`, artifact `xuloc/.xupm/build/xuloc` |
| `xuloc/src/ast.xulo` | One generic `Node` shape plus the stable tree renderer |
| `xuloc/src/lexer.xulo` | UTF-8 scalar scanner: tokens, spans, line/column, comments, literals, operators |
| `xuloc/src/parser.xulo` | Parser state (`Parser`), `parse`/`parse_expression` entries, and the submodule tree |
| `xuloc/src/parser/*.xulo` | Procedural parser modules: `tokens`, `nodes`, `template`, `types`, `decls`, `stmts`, `exprs`, `primary`, `control`, `patterns` |
| `xuloc/src/checker.xulo` | Name and type checking for the Gate 3 corpus: functions, bindings, control flow, lists, optionals, calls |
| `xuloc/src/backend.xulo` | Codegen entry: `compile`, declaration collection, emitter state, naming and literal text |
| `xuloc/src/backend/*.xulo` | Procedural emission modules: `bk_util`, `bk_state`, `bk_ns` (module namespaces), `bk_emit`, `bk_stmt`, `bk_expr`, `bk_call`, `bk_pattern` |
| `xuloc/runtime/runtime.c` | Unity entry point included by generated programs; assembles the `runtime_*.inc` modules in dependency order |
| `xuloc/runtime/runtime_internal.h` | Shared `XVal`, container, and module-dependency definitions |
| `xuloc/runtime/runtime_*.inc` | One module each: tagged values/containers, operators, rendering, method dispatch, `std::regex`, `std::fs`/`std::os`/`std::process`, `std::io`'s readers and terminal probes (its writes live with the renderer), `std::net`'s TCP sockets, `std::json`, `std::path`, `std::math`, `std::fmt`, `std::time`, `std::process::exec` |
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

The parser accepts the whole example corpus (T9): postfix `?` (with the
ternary/postfix disambiguation from `spec/expressions/operators.md`), arrow
closures, call-site type arguments, literal types, tuple position chains
(`p.0.1`), chained assignment (`a = b = 3`), function-macro calls, `lock`,
`spawn`, and service spawn. Binary operators use precedence climbing instead of
one function per level, so deeply nested `match` expressions stay under the
interpreter's 128-call limit; `std/net.xulo` and `std/http.xulo` now scan
cleanly. `crates/xulo-cli/tests/stage1_examples_baseline.tsv` is empty to keep
that ratchet.

The parser is deliberately syntactic. `xuloc check` adds the first semantic
pass: function signatures are collected before bodies, local bindings are
scoped, calls are checked for arity and argument types, `if`/`while` conditions
must be `bool`, `for` iterables must be `List`, returns must match the declared
return type, and operators check their operand types. The fixed corpus in
`xuloc/corpus/` is the Gate 3 evidence; `crates/xulo-cli/tests/stage1.rs`
requires stage 1 and stage 0 to agree on accept/reject for every file.

The checker is not yet a full bootstrap checker. Inline modules are modeled:
their functions and constants are collected in a module namespace, `use`
aliases/groups/globs and `pub use` re-exports resolve, `super` reaches a root
child's parent (including the package root), and module bodies are checked.
A file-backed module is opaque (the checker has no loader), so calls into it
leave `CheckResult.complete = false` instead of producing a false "not found";
`use type` from such a module marks the name as a foreign type, and operations
on the resulting `unknown` type stay unchecked. Simple structs are modeled:
field types, constructors with named or positional fields, and member reads.
Non-generic enums are modeled with payload constructors and `match` arms
(variant, literal, type, binder, wildcard, and guard patterns). Template
strings check their interpolated expressions, an untyped object literal gets
an anonymous shape so its member reads are checked, tuples carry a `(T1,T2)`
type through literals, annotations, positional reads/writes, and destructuring,
`type` aliases expand on demand, inherent `impl Type` methods bind `self` and
are callable with arity/type checks, default parameters relax the minimum
arity, optional struct fields may be omitted, `Result`/`Error` are prelude
types, `Map<K, V>`, `Set<T>`, and `Array<T, N>` carry their element/key types
through literals, constructors, members, indexing, iteration, and spread,
function types `fn(…)->…` and closures bind their parameters and returned
type, union types `A | B` accept either member and `is` / `is not` narrow the
tested binding in `if` branches, heterogeneous literals infer `unknown`, an
external `use path::*` keeps bare names opaque, and
ranges, optional chaining, null narrowing, `try`, `await`, block
expressions, and assignment expressions are checked. Traits, generic impls,
generics, and std calls still report E0003 or leave
`complete = false` ("not supported / not modeled") rather than being silently
accepted. `build` runs the checker and stops only on a
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

4. **Stage 1's source avoids postfix `?`.** The parser now produces a `Try`
   node and disambiguates it from the ternary, but the checker and C backend
   do not lower propagation yet. Stage 1's own code still matches `Result`
   explicitly.

5. **Stage 1's source avoids call-site type arguments.** The parser records
   them as a `TypeApply` node; the checker and backend ignore the node, so the
   compiler's own source calls monomorphic functions and constructors only.

6. **Stage 1's source uses `fn(...) { ... }` closures.** Arrow closures
   (`(x: number): number => …`, `x => …`) parse into an `ArrowClosure` node and
   the backend lowers both forms, but stage 1's own source keeps the `fn`
   spelling for readability.

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
    `if`/`while`/`for`/`match`, `defer` (each block's operands run in reverse
    registration order at fallthrough, `return`, `break`, `continue`, and
    `?`, after the block value is determined), calls, fields, indexing,
    lists, tuples (including positional `p.0` reads, writes, and optional
    reads), object literals (and their spread), typed `Map<K, V>{ … }`
    literals,
    `let` object/tuple destructuring, struct/enum/literal/or/range/type
    patterns with guards, templates, operators, `??`, `is`, `?.`, postfix
    `?` (propagate `Result::Err` with `return`), `Error("…")`, assignment in
    expression position (`a = b = 3`), `shared` as a pass-through unary,
    closures (function literals and arrow closures, including first-class
    values, calls through any expression, mutable captures, and returned
    closures), `#[derive(Debug)]` rendering, the `Map`/`Set` members (`size`,
    `get`, `set`, `has`, `remove`, `keys`, `values`, `entries`, `insert`),
    iteration over a map's keys or a set's elements, `Map::from_entries`,
    `Set::from_list`, `Array::filled<T, N>` with a literal or const-generic
    length, const generic parameters (an explicit call-site argument is
    appended to the call and bound as a function-local value), `lock`
    (evaluates its block), `spawn` (runs its block immediately in the compiled
    model), `spawn.process Type { … }` services (`send` dispatches to
    `on_message`, `stop` is a no-op), and the `Task::all`/`resolve`/`race`/
    `timeout` combinators over already-settled values, the
    `std::fs`/`std::os`/`std::path`/`std::hint`/`std::math`/`std::json` calls
    stage 1 uses, all of `std::process` (`args`/`exit`, `run`/`exec`/`launch`
    with optional input and timeout, the `Command` builders and setters, and
    `Child::wait`/`kill`/`pid`), all of `std::io` (the four writes, `flush`,
    the two tty probes, and `read_line`/`read`/`read_n`), the `std::time`
    clocks, `sleep`, and UTC calendar (`format`, `parse`, `parts`,
    `from_parts`, `weekday`), all of `std::fmt::format` (every verb,
    flag, width, precision, the `Display`/`Debug` consultation, and the
    import forms), `std::regex` (`RegExp::compile` with `i`/`g`/`m`/`s`/
    `u`, `test`/`find`/`find_all`, named and numbered groups, `string.replace`
    templates and closures, and `string.split`), all of `std::net` (TCP
    listen/connect/accept, read/write with optional timeouts, shutdown,
    close, and local/peer addresses over opaque handles), `std::http`
    (`listen`/`serve`, `fetch`/`request`, and the `Server`/`Request`/
    `Response` field tables with header/query/form lookup and the
    `text`/`json` shorthands), and `extern "C"`
    imports (`i8`–`i64`, `u8`–`u32`, `usize`/`isize`, `f32`/`f64`, `bool`,
    `unit`, and `Ptr`/`Ptr?`, with named-argument matching and reordering).
    `pub extern "C" fn` exports are lowered as ordinary functions, and a
    named function used as a value (`logging(route)`) gets a forwarding
    closure. Call-site
    type arguments are
    dropped because the backend is dynamically typed; a const argument is the
    exception, since the lowered body uses its value. Direct user-function
    calls apply named arguments and default values; a default expression is
    evaluated at the call site, so references to earlier parameters are
    outside the subset. Module namespaces are
    preserved: every file-backed and inline module gets its own scope, C
    symbols are mangled by module path, and `alias::fn` / `m::CONST` /
    `super::…` / `self::…` paths resolve through the module tree, `use`
    aliases, groups, globs, and `pub` re-exports (see `bk_ns.xulo`).
    `Trait::method(receiver, …)` lowers to runtime method dispatch on the
    receiver's struct type; `async` functions lower as ordinary functions and
    `await` is the identity. `u64` annotations and integer literals above
    `i64::MAX` are also
    `E0005`, because the bootstrap runtime has one signed 64-bit slot and does
    not tag `u64` (drift-governance D-04); a two-part path call that is neither
    a user function, a known enum variant, a trait method, nor a known static
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

19. **The bootstrap runtime is deliberately dynamic.** `xuloc/runtime/` (entry
    `runtime.c`) represents every value as a tagged `XVal`, keeps structs as field tables,
    dispatches methods through a registry, and leaks memory (the compiler is a
    one-shot process). Struct values are reference-shaped, so the R8
    value-copy table is not reproduced. This keeps the first backend small; the
    Rust stage 0 runtime remains the product runtime.

## Known gaps (tracked, not hidden)

- Integral arithmetic traps on overflow and division by zero with stage 0's
  `panic: <message>` text, floats render in stage 0's plain-decimal shortest
  form, and `u64` is an `E0005` rejection; these close drift-governance
  D-01–D-04 (`crates/xupm/tests/stage1_build.rs::stage1_c_backend_traps_like_the_stage0_runtime`,
  `::stage1_c_backend_matches_stage0_float_formatting`, and
  `::stage1_rejects_u64_in_the_bootstrap_subset`).
- `check` is a first semantic pass over the generic AST, not a typed tree. It
  covers the Gate 3 corpus, including inline modules, module `use` paths,
  simple struct construction/member reads, non-generic enum construction and
  `match`, template interpolation, untyped object literals, tuple
  expressions/types, simple `type` aliases, inherent methods, builtin
  `Result`/`Error`, `Map<K, V>`/`Set<T>`/`Array<T, N>` (with their member and
  static constructors), list/object spread, function types and closures,
  union types and `is` narrowing, external glob imports,
  ranges, optional chaining, and
  null narrowing; file-backed
  modules are opaque, and traits, generic impls, generics,
  and the remaining pattern forms still report E0003.
  `build` runs `check` first and
  refuses only a *confirmed* error: `CheckResult.complete` is `false` whenever
  the checker reached a construct, type, or call it does not model, so a
  partial analysis never rejects a program that code generation could handle.
- The C backend and the self-host loop are implemented (Gates 5–6), but the
  backend's subset is the bootstrap subset listed above. The runtime covers
  the `std` surface stage 1's own source needs plus `std::path`, `std::hint`,
  `std::os`, `std::math`, `std::fmt`, `std::regex`, `std::net`, `std::process`,
  `std::http`, `std::json`, and `std::fs`
  (its `read_dir` / `walk` / `realpath` are POSIX-only). Object literals lower
  to runtime objects, so
  member access and `{ ...base, k: v }` work. `Map` and `Set` are first-class
  in the bootstrap runtime: brace and typed map literals, the member methods,
  key/element iteration, structural set equality, and the static constructors
  lower to `XT_OBJ` (map) and `XT_SET` (set) values. `std::time` is lowered
  (clocks, `sleep`, and the UTC calendar over pure integer date arithmetic),
  and so are `std::fmt::format`, `std::regex`, `std::net`, `std::process`, and
  `std::http`.
- Destructuring (`let (a, b)`, `let { x, y }`), struct/enum/literal/or/range/
  type patterns with guards, and `#[derive(Debug)]`'s `Type(field: value)`
  rendering are lowered. A struct without the derive still renders as
  `Type{ field: value }`, which is stage 1's own fallback form.
- `f32` arithmetic runs in binary64 in the bootstrap runtime; stage 0 rounds
  to binary32 (T12), so an example that depends on that rounding can differ
  (`types/float_math.xulo`).
- The parser accepts postfix `?`, arrow closures, call-site type arguments,
  function-macro calls, `spawn`, and `lock`. The backend lowers `?`, closures
  (including arrow closures and captured bindings), `spawn`/`lock` in the
  synchronous model, and drops call-site type arguments, but macro calls still
  report `E0005`; attribute macros and macro expansion remain outside the
  bootstrap subset, and macro bodies are skipped as token trees.
- The backend builds every example file that stays inside the bootstrap
  subset, including the file-backed, inline, nested, glob, and re-export
  module cases. `errors/type_error.xulo` is the expected reject; the remaining
  failures need macros or `u64`.
  `crates/xupm/tests/stage1_build.rs` pins the `lexical`,
  `propagation`, `arrays`, `std/time`, `std/io`, `std/io_extra`, `std/fmt`,
  `std/regex`, `std/net`, `std/net_shutdown`, `std/process`, `std/http`,
  `functions/closures_hof`, `functions/defer`, `types/aliases`,
  `types/generics_impl`, `concurrency/spawn_lock`, `ffi/ffi`, and every
  `modules/` case against their sidecars (stdout and stderr).
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
