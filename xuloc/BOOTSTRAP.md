# Stage 1 bootstrap notes

Stage 1 is a lexer, parser, and first semantic checker for the bootstrap
subset, written in Xulo and executed by stage 0. Gates 1–3 prove that the
language is usable for its own front end; code generation and the self-host
loop (Gates 5–6) are the next stage.

## Layout

| File | Role |
|------|------|
| `xuloc/ast.xulo` | One generic `Node` shape plus the stable tree renderer |
| `xuloc/lexer.xulo` | UTF-8 scalar scanner: tokens, spans, line/column, comments, literals, operators |
| `xuloc/parser.xulo` | Recursive-descent parser: headers, declarations, statements, expressions, patterns, types |
| `xuloc/checker.xulo` | Name and type checking for the Gate 3 corpus: functions, bindings, control flow, lists, optionals, calls |
| `xuloc/corpus/accept/` | Files stage 1 and stage 0 must both accept |
| `xuloc/corpus/reject/` | Files both must reject, with a coded semantic diagnostic |
| `xuloc/main.xulo` | CLI: `tokens`, `parse`, and `check` modes, diagnostics, exit status |
| `xuloc/tests.xulo` | Stage 1's own `#[test]` functions, including self-parse and corpus checks |

`crates/xulo-cli/tests/stage1.rs` drives the same files through the stage 0
binary, and `crates/xupm/tests/cli.rs` verifies that `xupm test` runs the
`#[test]` functions in `xuloc/tests.xulo`.

`ast`, `lexer`, `parser`, and `checker` are modules, so check them through an entry point
(`xuloc/main.xulo` or `xuloc/tests.xulo`). Checking `xuloc/parser.xulo` on
its own cannot resolve the sibling `ast` module, by design.

## Running

`xulo run` does not forward script arguments, so stage 1 accepts either a
compiled binary's argv or two environment variables:

```bash
# Interpreter path (Gate 1/2)
XULO_STAGE1_MODE=tokens XULO_STAGE1_FILE=examples/fibonacci.xulo \
  target/debug/xulo run xuloc/main.xulo

XULO_STAGE1_MODE=parse XULO_STAGE1_FILE=xuloc/parser.xulo \
  target/debug/xulo run xuloc/main.xulo

XULO_STAGE1_MODE=check XULO_STAGE1_FILE=xuloc/corpus/accept/basics.xulo \
  target/debug/xulo run xuloc/main.xulo

# Native path once a binary exists
xuloc tokens examples/fibonacci.xulo
xuloc parse examples/fibonacci.xulo
xuloc check xuloc/corpus/accept/basics.xulo
```

The default mode is `parse`. Diagnostics use the stage 0 codes and one-line
shape `<file>:<line>:<column>: error[E…] <kind>: <message>` (`E0001` lexical,
`E0002` syntax, `E0004` I/O). Semantic failures use `E0101` (missing name),
`E0201` (type mismatch), `E0202` (argument mismatch), `E0211` (no common type),
`E0213` (not iterable), and `E0215` (not indexable). All failures exit 1.
`check` prints `no errors` on success, matching `xulo check`.

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
(L10), modules and `use` (L12), optionals and `??`/`?.` types (L16), and
`Result`/`Error` matching (L19). It also recognises `defer`, `async`/`await`,
attributes, and trait/impl declarations well enough to skip through them.

The parser is deliberately syntactic. `xuloc check` adds the first semantic
pass: function signatures are collected before bodies, local bindings are
scoped, calls are checked for arity and argument types, `if`/`while` conditions
must be `bool`, `for` iterables must be `List`, returns must match the declared
return type, and operators check their operand types. The fixed corpus in
`xuloc/corpus/` is the Gate 3 evidence; `crates/xulo-cli/tests/stage1.rs`
requires stage 1 and stage 0 to agree on accept/reject for every file.

The checker is not yet a full bootstrap checker. `match`, enums, structs,
traits, impls, generics, modules, closures, and components report E0003
("not supported yet") rather than being silently accepted.

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

8. **No component calls.** An identifier followed by a `{ ... }` child block is
   not parsed as `ComponentCall`. Stage 1 emits tokens and a tree from ordinary
   files and does not parse `.xulo` UI examples.

9. **No destructuring binding in stage 1's source.** The parser skips a
   `let { ... }` or `let ( ... )` pattern, but stage 1 itself binds one name at
   a time and compares optionals to `null` or uses `??`.

10. **No struct patterns in `match`.** Variant payloads and literals are
    parsed; stage 1 matches `Result::Ok(value)`/`Result::Err(_)` and plain names
    only.

11. **Match arms carry expressions or blocks, not statements.** A `return` in
    an arm is therefore written `=> { return ... }`. This is the language rule,
    not a stage 1 extension; it is called out because it is easy to trip over
    while writing bootstrap code.

12. **String scanning is character based.** The lexer splits the source once
    with `split("")`, then indexes the resulting list, so token spans are O(1)
    per character. It does not use `string.code_at` inside the hot loop.

13. **`mut self` is the mutation tool.** `Lexer` and `Parser` keep their cursor
    and error state in struct fields and mutate them with `mut self` methods;
    recursive calls such as `self.expr()` work on the caller's binding. This is
    the proven L9/L10 pattern stage 1 relies on.

14. **Scope exit does not use `pop`/`truncate`.** `checker.xulo` records a
    scope id per binding and keeps an active-scope count. Leaving a block only
    decrements the count, so old bindings become invisible without list
    removal. This keeps the checker inside the AOT-lowered list operations.

15. **Types are canonical strings.** `checker.xulo` represents `number`,
    `string`, `bool`, `unit`, `unknown`, `null`, `List<T>`, and `T?` as strings.
    Type comparison is therefore string equality, avoiding the structural
    equality gap for tuples/lists and keeping the checker small.

16. **AOT-safe members only.** The native build (Gate 4/B8a) does not lower the
    list members `.includes` and `.join`; the lexer uses an explicit keyword
    loop and a `StringBuilder` loop for token text. The interpreter accepts
    both forms, but stage 1's own source stays inside the subset its native
    binary can execute.

## Known gaps (tracked, not hidden)

- `check` is a first semantic pass over the generic AST, not a typed tree. It
  covers the Gate 3 corpus; structs, enums, traits, impls, generics, modules,
  closures, components, and `match` still report E0003.
- Code generation and the self-host loop (Gates 5–6) are not implemented.
- Postfix `?`, arrow closures, call-site type arguments, components, macros
  beyond attributes, and `spawn`/`lock` are outside the parser's accepted
  subset.
- The parser recovers only by stopping at the first error. That is enough for
  Gate 1/2 diagnostics; stage 1's checker work will need recovery.
- The tests read `xuloc/*.xulo` relative to the repository root, like the
  rest of the repository's smoke tests.

## Verification

```bash
./target/debug/xulo check xuloc/main.xulo
./target/debug/xulo test xuloc/tests.xulo
./target/debug/xupm test xuloc/tests.xulo
XULO_STAGE1_MODE=check XULO_STAGE1_FILE=xuloc/corpus/accept/basics.xulo \
  ./target/debug/xulo run xuloc/main.xulo
cargo test -p xulo-cli --test stage1
cargo test -p xupm --test cli

# Gate 4/B8a: native stage 1 matches the interpreter.
XULO_RUNTIME_LIB="$PWD/target/release/libxulo_runtime.a" \
  target/release/xupm build xuloc/main.xulo --release
XULO_AOT_DIFFERENTIAL=1 XULO_RUNTIME_LIB="$PWD/target/release/libxulo_runtime.a" \
  cargo test -p xupm --test aot_differential stage1_binary_matches_the_interpreter
```
