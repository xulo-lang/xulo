# Stage 1 bootstrap notes

Stage 1 is a lexer and parser for the bootstrap subset, written in Xulo and
executed by stage 0. It is intentionally a parser, not a compiler: Gate 1 and
Gate 2 are about proving that the language is usable for its own tooling
before stage 1 grows a checker or code generator.

## Layout

| File | Role |
|------|------|
| `xuloc/ast.xulo` | One generic `Node` shape plus the stable tree renderer |
| `xuloc/lexer.xulo` | UTF-8 scalar scanner: tokens, spans, line/column, comments, literals, operators |
| `xuloc/parser.xulo` | Recursive-descent parser: headers, declarations, statements, expressions, patterns, types |
| `xuloc/main.xulo` | CLI: `tokens` and `parse` modes, diagnostics, exit status |
| `xuloc/tests.xulo` | Stage 1's own `#[test]` functions, including the self-parse test |

`crates/xulo-cli/tests/stage1.rs` drives the same files through the stage 0
binary, and `crates/xupm/tests/cli.rs` verifies that `xupm test` runs the
`#[test]` functions in `xuloc/tests.xulo`.

`ast`, `lexer`, and `parser` are modules, so check them through an entry point
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

# Native path once a binary exists
xuloc tokens examples/fibonacci.xulo
xuloc parse examples/fibonacci.xulo
```

The default mode is `parse`. Diagnostics use the stage 0 codes and one-line
shape `<file>:<line>:<column>: error[E…] <kind>: <message>` (`E0001` lexical,
`E0002` syntax, `E0004` I/O) and exit with status 1.

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

The parser is deliberately syntactic. It does not resolve names, infer types,
check exhaustiveness, or evaluate constants. Gate 3 and later add those jobs.

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

14. **AOT-safe members only.** The native build (Gate 4/B8a) does not lower the
    list members `.includes` and `.join`; the lexer uses an explicit keyword
    loop and a `StringBuilder` loop for token text. The interpreter accepts
    both forms, but stage 1's own source stays inside the subset its native
    binary can execute.

## Known gaps (tracked, not hidden)

- `parse` does not build a typed tree, so Gate 3 (self-check) needs a semantic
  pass over this shape or a richer AST.
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
cargo test -p xulo-cli --test stage1
cargo test -p xupm --test cli

# Gate 4/B8a: native stage 1 matches the interpreter.
XULO_RUNTIME_LIB="$PWD/target/release/libxulo_runtime.a" \
  target/release/xupm build xuloc/main.xulo --release
XULO_AOT_DIFFERENTIAL=1 XULO_RUNTIME_LIB="$PWD/target/release/libxulo_runtime.a" \
  cargo test -p xupm --test aot_differential stage1_binary_matches_the_interpreter
```
