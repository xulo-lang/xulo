# Changelog

All notable changes to Xulo are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Aligned with `spec@815edad` (specification 0.2.0), which removes the component
layer from the language.

### Removed

- The component/UI language layer: `View`, `@State`, `@Store`, `@Effect`,
  `@Environment`, component declarations and blocks, `$` binding arguments, and
  the `@` token.
- `main` must now return `unit`; `fn main(): View` is rejected.
- Diagnostic codes `E0701`–`E0799` (reserved and never reused).
- The UI crates (`xulo-framework`, `xulo-ui`, `xulo-ui-wasm`,
  `xulo-render-core`, `xulo-renderer-terminal`, `xulo-renderer-skia`,
  `xulo-renderer-webview`) and the CLI `--render`, `webview`, and `skia`
  options/features.

UI construction remains possible with ordinary `struct`, `enum`, and `fn`
values, and is left to a future standard-library `std/ui` module.

### Changed

- Stage 1's checker now registers trait method signatures: `Area::area(recv)`
  returns the trait's declared type (and takes the receiver as its first
  argument), so `let n: number = Area::area(rect)` is checked. Trait impl
  bodies and dispatch verification stay outside the pass.
  `xuloc/corpus/accept/traits.xulo` covers two traits;
  `reject/trait_return_mismatch.xulo` pins the mismatch.
- Stage 1's parser keeps an `impl<T, …>` parameter list, and the checker now
  instantiates `impl<T> Type<T>` methods: the receiver type supplies `T` for
  the method's parameter and return templates (`Box<int>.get()` returns
  `number`), and method-level parameters infer from the call arguments.
  `xuloc/corpus/accept/generic_methods.xulo` pins the behavior.
- Stage 1's checker now instantiates user-defined generic structs:
  `Palette<int>` substitutes field types (`items: List<int>`) on reads and in
  constructor arguments, constructor calls infer the struct's type parameters,
  generic inherent methods remain callable, and assignability compares the
  arguments structurally. `xuloc/corpus/accept/generic_structs.xulo` covers
  construction, fields, and an `impl<T>` method.
- Stage 1's checker now instantiates user-defined generic enums:
  `Option<int>` keeps its payload types through `Option::Some(1)`,
  `Option::None`, and `match` bindings; constructor arguments infer the enum's
  type parameters, and assignability compares generic arguments structurally.
  `xuloc/corpus/accept/generic_enums.xulo` covers inference and binding.
- Stage 1's checker now infers type arguments for generic function calls from
  the actual arguments (`first([1, 2])` returns `number`,
  `twice("x")` returns `List<string>`), while an unresolved parameter stays
  `unknown`. Signature templates keep `$T$` placeholders and substitution is
  applied to the return type. `xuloc/corpus/accept/generics.xulo` covers
  typed generic calls; `reject/generic_call_mismatch.xulo` pins the mismatch.
- Stage 1's checker now carries `Result<T, E>` payload types instead of
  treating every `Result` as opaque: `type_of`, `assignable`, `Result::Ok` /
  `Result::Err` constructors, and `match` bindings use the instantiated
  ok/error types, and postfix `?` yields the ok type (`E0217` when the operand
  is neither `T?` nor a `Result`). Tuple types are assignable element-wise.
  `xuloc/corpus/accept/results.xulo` covers typed `?` and payloads;
  `reject/try_non_result.xulo` pins the diagnostic.
- The stage 1 checker is now modular: `xuloc/src/checker.xulo` keeps the result
  and signature records, the `Checker` state, and the `check` entry point,
  while the passes live in `xuloc/src/checker/` (`ck_util`, `ck_diag`,
  `ck_module`, `ck_types`, `ck_decls`, `ck_stmt`, `ck_expr`). Each pass is a
  free function taking the checker state, matching the backend's procedural
  split, because Xulo methods cannot cross module boundaries. Behavior is
  unchanged; the self-parse test now walks `xuloc/src/` recursively.
- The conformance triage now classifies the 13 examples that write statements at
  module scope as **fragments that assume a function body**, not as a
  module-scope-statement parser gap: `spec/grammar.md` defines `Program` as
  `{ HeaderItem } { Declaration } [ Entry ]`, so module scope holds
  declarations only. The fix is a self-contained corpus that wraps the snippet,
  tracked on the `crates/xulo-cli/tests/` side.
- The AOT differential now accepts `@aot-failure-shape <platform>:` (T13 option
  c). Examples that declare it must fail on both paths with the same normalized
  runtime message; stdout and diagnostic decoration are not compared, so the
  interpreter keeps its `error[E0006]` + source snippet and the native binary
  keeps `panic: …`. `errors/division_by_zero.xulo` and
  `errors/runtime_failure.xulo` use the directive instead of `@no-aot`.
  Regression:
  `crates/xupm/tests/aot_differential.rs::failure_shape_normalizes_both_renderings`.
- The `README` and `learn/book` guide now describe the UI-less 0.2.0 language:
  `mod` / `use` / `pub use` instead of `import ... from`, `Result<T, E>` and `?`
  instead of `try` / `catch` / `throw`, the specification's 61-keyword and
  reserved-word tables, `int`/`float`/`number` distinctions, and the current
  stage 1 self-hosting status.
- The formatter/editor token layer now classifies exactly the specification's
  closed keyword set and reserved list (spec `lexical-structure.md`):
  `print`/`println` are ordinary prelude names, `try`/`catch`/`throw`/
  `import`/`from` lex as identifiers, and `defer`/`move`/`self`/`spawn`/
  `super` are keywords rather than reserved words.

### Added

- Stage 1's checker now honors default parameters: a call may omit trailing
  defaulted arguments, so `examples/functions/params.xulo` reports
  `no errors`.
- Stage 1's checker now treats bare names bound by an external `use` path
  (`use std::http::{Request, Response}`, `use std::os::exec_path`) as opaque,
  and a named function used as a value gets its `fn(...)->...` type.
  `examples/std/http.xulo` reports `no errors`; the example check baseline
  drops from 2 to 1 (only the intentional `errors/type_error.xulo` remains).
  `xuloc/corpus/accept/imports.xulo` pins external group imports and function
  values.
- Stage 1's checker now skips macro definition bodies (`#[macro]`,
  `#[macro_attr]`, `#[macro_derive]`), treats a macro invocation as `unknown`,
  and keeps unresolved names opaque when a macro is declared (a macro may
  generate functions, types, or members). `examples/macros/macros.xulo`
  reports `no errors`; the example check baseline drops from 3 to 2.
- Stage 1's checker now treats generic type/const parameters (`T`, `const N:
  int`) as `unknown` instead of unknown types, binds const parameters in the
  body, and accepts `trait` declarations with trait dispatch left opaque.
  `examples/functions/generics`, `types/generics_impl`, and
  `types/trait_shapes` report `no errors`; the example check baseline drops
  from 6 to 3. `xuloc/corpus/accept/generics.xulo` pins the behavior.
- Stage 1's checker now models `extern "C"` declarations: the `as` alias is
  the local binding, a declaration without a return type is `unit`, `Ptr` is
  an opaque address type, and an optional `Ptr?` parameter may be omitted.
  `examples/ffi/ffi.xulo` reports `no errors`; the example check baseline
  drops from 7 to 6. `xuloc/corpus/accept/extern.xulo` and
  `reject/extern_argument.xulo` pin the behavior.
- Stage 1's checker now models `async`/`await`, `spawn` (default, thread,
  process, and `on(target)`), `lock`, `shared`, and the service `reply`
  intrinsic. `examples/concurrency/async_tasks`, `spawn_lock`, and
  `testing/test_functions` now report `no errors`; the example check baseline
  drops from 10 to 7. `xuloc/corpus/accept/concurrency.xulo` pins the behavior.
- Stage 1's checker now models union types (`A | B`), `is`/`is not` tests and
  branch narrowing, heterogeneous literals as `List<unknown>`, and external
  glob imports (`use std::fmt::*`) as opaque. A statement-position `match` may
  mix arm types. `examples/types/aliases`, `std/fmt`, `std/json`, and
  `std/io_extra` now report `no errors`; the example check baseline drops from
  14 to 10. `xuloc/corpus/accept/unions.xulo` and
  `reject/union_mismatch.xulo` pin the behavior.
- Stage 1's checker now models function types (`fn(A, B)->R`), function
  literals, and arrow closures: parameters and the declared return type are
  checked, captures see the enclosing scope, and values can be called through
  any expression (`fs[0]()`, `make()(1)`). `examples/functions/closures_hof`
  and `examples/std/regex` now report `no errors`; the example check baseline
  drops from 16 to 14. `xuloc/corpus/accept/closures.xulo` pins the behavior.
- Stage 1's checker now models `Map<K, V>`, `Set<T>`, and `Array<T, N>`:
  annotation and literal types, the `Map`/`Set` members (`size`, `get`, `set`,
  `has`, `remove`, `keys`, `values`, `entries`, `insert`), `Set::from_list` and
  `Array::filled<T, N>`, key/value indexing, iteration over map keys and set
  elements, and list/object spread. `examples/collections/*` and the new
  `xuloc/corpus/accept/collections.xulo` pin the behavior end to end; the
  example check baseline drops from 20 to 16 failures.
- Stage 1's checker now treats `Result`/`Error` as prelude types and models
  `try`, `await`, block and assignment expressions, ranges (`a..<b`, `a..=b`),
  optional chaining (`?.`), and `!= null` / `== null` narrowing. `unknown`
  operands are accepted instead of flagged. This moves `for`, optional
  chaining, propagation, and several `std` examples to `no errors`;
  `xuloc/corpus/accept/results.xulo` pins builtin `Result`/`Error` end to end.
- Stage 1's C backend now lowers tuple positional members (`p.0` reads and
  writes, plus `p?.0` through `xr_index_opt`). `examples/types/tuples.xulo`
  builds and runs with stage 1 and is pinned in `stage1_build`.
- Stage 1's C backend now applies defaults and named arguments for direct user
  function calls, so `examples/functions/params.xulo` builds and runs
  (`2 / 20 / hi! / hey? / shared`) and is pinned in `stage1_build`.
- Stage 1's checker now models inherent `impl Type` methods: `self` is bound
  to the owner type, method bodies are checked, and method calls validate
  arity and arguments (including chains). Optional struct fields may be
  omitted, typed object literals are checked against their struct, and struct
  patterns bind their fields. `examples/types/structs.xulo` now reports
  `no errors`; `xuloc/corpus/accept/methods.xulo` is pinned end to end.
- Stage 1's checker now expands simple `type` aliases (including module
  aliases imported through `use type`/`use`), with on-demand expansion and a
  cycle guard. `xuloc/corpus/accept/aliases.xulo` is checked by both compilers
  and built/run by stage 1; `xuloc/corpus/reject/alias_mismatch.xulo` pins the
  `E0201` mismatch.
- Stage 1's checker now models tuples: `(T1,T2)` types, tuple literals,
  positional reads (`p.0`), tuple member assignment, and tuple destructuring.
  `examples/basics/bindings.xulo`, `examples/patterns/destructuring.xulo`, and
  `examples/types/tuples.xulo` now report `no errors`.
- Stage 1's checker now models untyped object literals with an anonymous
  shape, so `${user.name}` and `user.age` are checked instead of rejected;
  object destructuring binds fields from that shape, while tuple destructuring
  binds `unknown`. `examples/basics/templates.xulo` now reports `no errors`,
  a spread or other form that needs an expected type stays `complete = false`,
  and `examples/basics/bindings.xulo` still builds through stage 1.
- Stage 1's checker now checks template expressions (every `${…}` is
  type-checked, the template is a `string`) and binds generic-enum payload
  names to `unknown` so the arm body still checks. This lets
  `examples/types/enums_match.xulo` report `no errors` while staying
  `complete = false`, and `xuloc/corpus/accept/enums.xulo` now builds and runs
  a template interpolation end to end.
- Stage 1's checker now models non-generic enums and `match`: payload-free
  variant values, payload variant constructors, payload bindings, literal,
  type, binder, wildcard, and guard patterns, and arm result unification.
  `xuloc/corpus/accept/enums.xulo` is checked by both compilers and built/run
  by stage 1; generic enums stay `unknown`/incomplete rather than becoming
  false errors.
- Stage 1's checker now models simple structs: field types, constructors with
  named or positional fields, and member reads. Struct names resolve through
  the module tree, signatures and field types are resolved after `use`
  statements, and `use type` from an opaque/file-backed module becomes a
  foreign type whose operations stay unchecked (`complete = false`) rather
  than becoming false errors. `xuloc check examples/modules/visibility/main.xulo`
  and `examples/modules/imports/main.xulo` now report `no errors`. Regressions:
  `xuloc/corpus/accept/modules.xulo` (a module struct) and
  `xuloc/corpus/reject/struct_field_mismatch.xulo`.
- Stage 1's checker now models inline modules: it collects a module namespace,
  resolves `use` aliases/groups/globs, `pub use` re-exports, and `super`/`self`
  paths, and checks module bodies. `xuloc check examples/modules/inline/main.xulo`
  reports `no errors`, a module-body type error is reported with its stage 0
  code (`E0203`), and a file-backed module stays opaque (`complete = false`)
  instead of becoming a false "not found". Regression corpus:
  `xuloc/corpus/accept/modules.xulo` and
  `xuloc/corpus/reject/module_return_mismatch.xulo`.
- Stage 1's C backend now keeps module namespaces instead of flattening them.
  Every file-backed or inline `mod` gets a scope, its functions and constants
  get module-mangled C symbols (`f_5inner6detail_quadruple`), and path/name
  resolution walks the module tree through `self`, `super`, `use` aliases,
  groups, globs, and `pub use` re-exports. Nested inline modules no longer
  collide in the generated C (`examples/modules/inline/main.xulo` is pinned in
  `crates/xupm/tests/stage1_build.rs`), and stage 1 still self-hosts.
- Stage 1's C backend now lowers `std::http` over a new `runtime_http.inc`:
  `listen`/`serve` with a function-value handler, `fetch`/`request` clients,
  and the `Server`/`Request`/`Response` field tables with case-insensitive
  header lookup, exact query/form lookup, percent decoding, and the
  `Response.text`/`Response.json` shorthands. The backend also lowers named
  functions used as values (`logging(route)`) as forwarding closures and
  static dot calls (`Response.text`). `examples/std/http.xulo` now compiles
  and matches the interpreter byte for byte, including its self-launched
  server, three client exchanges, and the captured server log. Stage 1 build
  coverage reaches 61 of 66 example files.
- Stage 1's C backend now lowers the full `std::process` surface: `run`,
  `exec`, and `launch` with optional input and timeout; the `Command`
  `shell`/`direct` constructors and pure `input`/`cwd`/`env`/`timeout`
  setters; and `Child::wait`/`kill`/`pid`. Both streams are captured
  separately and decoded lossily, a budget kills an overrun child, and a
  spent handle refuses a second `wait`/`kill`. POSIX uses fork/exec with a
  non-blocking drain; Windows uses `CreateProcess` and `WaitForSingleObject`.
  Windows stdout/stderr now run in binary mode so captured CRLF text and the
  program's own lines match stage 0 byte for byte.
  `examples/std/process.xulo` builds (and runs on Unix); a cross-platform
  regression covers shell `run`, `Command::shell().run()`, `launch`/`wait`,
  and a spent handle. Stage 1 build coverage reaches 60 of 66 example files.
- Stage 1's C backend now lowers `extern "C"` imports and exported
  `pub extern "C" fn` bodies. The parser records import signatures, and calls
  convert arguments by the declared ABI type (integers, `usize`/`isize`,
  `f32`/`f64`, `bool`, `unit`, and opaque `Ptr`/`Ptr?` values), match named
  arguments by parameter name, and convert the return value back to a tagged
  value. `examples/ffi/ffi.xulo` now compiles and matches the interpreter;
  stage 1 build coverage reaches 59 of 66 example files.
- Stage 1's C backend now lowers `std::net` over a new `runtime_net.inc`:
  opaque handles with listener/stream kinds, TCP listen/connect/accept,
  read/write with optional millisecond timeouts, shutdown/shutdown_write,
  close, and local/peer addresses over IPv4/IPv6 and DNS names. Windows
  builds link `ws2_32`, and the runtime carries Winsock and POSIX socket
  paths. `examples/std/net.xulo` and `examples/std/net_shutdown.xulo` now
  compile and match the interpreter byte for byte; a regression test covers
  handle validation, the read-size domain, wrong-kind handles, and a second
  close. Stage 1 build coverage reaches 58 of 66 example files.
- Stage 1's C backend now lowers `std::regex` over a new `runtime_regex.inc`:
  a backtracking engine for literals, `.`, classes and escapes, groups
  (numbered, named, non-capturing), alternation, greedy/lazy and counted
  repetition, anchors, and the `i`/`g`/`m`/`s`/`u` flags; `RegExp::compile`
  returns `Result<RegExp, RegexError>` with the engine's parse-error text.
  `test`/`find`/`find_all`, `RegexMatch.group`, `string.replace` (templates
  and closures), and `string.split` are lowered. `examples/std/regex.xulo`
  now compiles and matches the interpreter byte for byte; a regression test
  adds alternation order, empty matches, dotall, multiline, lazy/counted
  repetition, named groups, word boundaries, and Unicode indices. Stage 1
  build coverage reaches 56 of 66 example files.
- Closures now box pattern bindings and capture only bindings in live scopes.
  This fixes a latent bug exposed by match-heavy programs: pattern bindings
  were declared as plain values while reads used cells, and locals from
  closed sibling scopes leaked into later closure capture lists.
- Stage 1's C backend now lowers `lock`, `spawn`, services, and the `Task`
  combinators in the compiled model: `lock target { … }` evaluates its block,
  a spawn block runs immediately (so `await` stays the identity), and
  `spawn.process Type { … }` builds a `Type` instance whose `send` dispatches
  to `on_message` and whose `stop` is a no-op. `Task::all`/`resolve`/`race`/
  `timeout` read already-settled values, and `reply` is the handler's value.
  `examples/concurrency/spawn_lock.xulo` now compiles and matches the
  interpreter byte for byte, including repeated service messages. Stage 1
  build coverage reaches 55 of 66 example files.
- Stage 1's C backend now lowers const generic parameters: the parser marks
  `const N: int` as `ConstGenericParam`, call sites append explicit const
  arguments (a literal or an enclosing const parameter) after the value
  arguments, and the function body binds each one as a local. `Array::filled`
  accepts such a parameter as its length. `examples/types/generics_impl.xulo`
  now compiles and matches the interpreter; a regression test covers
  const-only, mixed type/const, and const-length array construction. Stage 1
  build coverage reaches 54 of 66 example files.
- Stage 1's C backend now lowers `std::fmt::format` over a new
  `runtime_fmt.inc`, including every verb (`v`/`s`/`q`/`t`/`c`, the integer
  and floating-point verbs), the flag/width/precision vocabulary, the
  `Display`/`Debug` consultation for user structs (with a `Format` carrying
  `precision` and `alternate`), `#[derive(Debug)]` fallback, and the
  qualified, imported, and aliased call forms. `examples/std/fmt.xulo` now
  compiles and matches the interpreter byte for byte; a regression test adds
  rounding, negative zero, alternate forms, quoting, multibyte width, and the
  extreme integer cases. Stage 1 build coverage reaches 53 of 66 example
  files.
- Stage 1's C backend now lowers `defer`: each block keeps a registration
  list, and its operands run in reverse registration order when the block
  leaves through fallthrough, `return`, `break`, `continue`, or `?`, after the
  block's value is determined. Nested blocks run innermost first, and a
  `panic` still stops before any defer runs. `examples/functions/defer.xulo`
  now compiles and matches the interpreter byte for byte; a regression test
  also covers loop `continue`/`break`, `?` propagation, nested blocks, and
  block-expression values. Stage 1 build coverage reaches 52 of 66 example
  files.
- Stage 1's C backend now lowers closures: function literals and arrow
  closures, first-class function values called through any expression,
  captured bindings (boxed into shared heap cells, so mutable captures and
  returned closures keep working), and nested closures. The bootstrap runtime
  gains an `XT_FN` value and a generated `xr_closure_dispatch` that maps a
  closure id to its body; a function value renders as `<function>` like stage
  0. `examples/functions/closures_hof.xulo` and `examples/types/aliases.xulo`
  now compile and match the interpreter, so stage 1 build coverage reaches 51
  of 66 example files, and both are pinned in the lowered-example test.
- Stage 1's C backend now lowers `std::time`: the wall and monotonic clocks,
  `sleep`, and the UTC calendar (`format`, `parse`, `parts`, `from_parts`,
  `weekday`) over a `runtime_time.inc` module that uses pure integer date
  arithmetic, so it does not depend on the host time zone or locale.
  `examples/std/time.xulo`, `math_time.xulo`, and `std.xulo` now compile and
  match the interpreter byte for byte. Stage 1 build coverage reached 49 of 66
  example files before the `std::io` readers landed.
- Stage 1's C backend now lowers the rest of `std::io`: the two tty probes and
  the three stdin readers (`read_line`, `read`, `read_n`) over a new
  `runtime_io.inc`. The readers decode strict UTF-8, report the first invalid
  byte's offset, distinguish `Ok(null)` at end of input from `Ok("")`, and
  `read_n` keeps an incomplete trailing scalar for the next call.
  `examples/std/io_extra.xulo` now compiles and matches the interpreter on
  stdout and stderr, so stage 1 build coverage reached 50 of 66 example files
  before closures landed. `examples/std/io.xulo` is now pinned alongside it.
- Stage 1's C backend lowers postfix `?` (propagating `Result::Err` with a
  `return`), `Error("…")`, assignment in expression position (`a = b = 3`),
  `shared` as a pass-through unary, and `Array::filled<T, N>` with a literal
  length. Call-site type arguments are dropped because the backend is
  dynamically typed, and Xulo's `\u{…}` escapes are translated to C's
  `\uXXXX`/`\UXXXXXXXX` form. The bootstrap runtime now expands `%g` exponent
  notation to plain decimal, so `1000.0` prints like stage 0. The stage 1
  build coverage test pins `basics/lexical.xulo`, `errors/propagation.xulo`,
  and `collections/arrays.xulo` end to end.
- Stage 1's parser now accepts the whole example corpus, so
  `crates/xulo-cli/tests/stage1_examples_baseline.tsv` is empty (66 files, no
  lexer or parser gaps). It gained postfix `?` with the specification's
  ternary/postfix disambiguation, arrow closures, call-site type arguments,
  literal types, tuple position chains (`p.0.1`), right-associative chained
  assignment, function-macro calls (bodies are skipped as token trees), `lock`,
  `spawn`, and service spawn. Binary operators were rewritten as a
  precedence-climbing loop, which keeps deeply nested `match` expressions
  (`std/net.xulo`, `std/http.xulo`) under the interpreter's 128-call limit.
  Regressions: `xuloc/src/tests.xulo::parses_postfix_propagation_beside_ternary`
  and the other parser tests there;
  `crates/xulo-cli/tests/stage1.rs::stage1_example_corpus_tracks_baseline`.
- Struct deconstruction in `match` (`Point(x, y)`) now works on all three paths:
  the checker validates the struct type and field count and binds each field
  with its declared type, the interpreter deconstructs the object by
  declaration order, and the AOT backend reads the fields through `ObjectGet`.
  The grammar gate's last `error-only:` row (`examples/coverage.tsv`,
  `StructPattern`) now points at `examples/types/structs.xulo`. Nested patterns
  inside a struct slot (`Shape::Dot(Point(x, y))`) remain unsupported and are
  rejected with a clear diagnostic.
- `examples/std/` now covers the 38 standard-library members the grammar gate
  could not see: `math_extra.xulo` (trig/log/`trunc`/`clamp`/`sign`, the integer
  bit ops, `random`), `fs_extra.xulo` (temporary creators, exclusive create,
  byte append, times/mode, `realpath`), `io_extra.xulo` (`eprint`, the tty
  probes, the EOF reads), `net_shutdown.xulo` (`sock_shutdown`), `exit` in
  `process.xulo`, and a `split` example in `regex.xulo`. Every one builds
  natively and prints what the interpreter prints.
- `crates/xulo-cli/tests/examples.rs::std_members_are_exercised_by_the_corpus`
  keeps that coverage honest: it walks every row of
  `std-spec/signature-index.md`, ignores comments, exempts `type …` rows, and
  fails when a callable member has no `std::<module>::<member>` call (or
  `.method(` for the `string.*` pseudo members). It skips when the `std-spec`
  submodule is not checked out.

### Fixed

- A `const` initializer is now checked as a constant expression. A call is
  `E0210` ("const initializer is not a constant expression"), and constant
  arithmetic reports `E0210` for division/remainder by zero, integer overflow,
  a negative exponent, or a shift amount outside `0..64`; references to another
  `const` and arithmetic/comparison over constants stay legal. Float constant
  arithmetic keeps IEEE-754 semantics. Regression:
  `crates/xulo-semantic/tests/semantic_tests.rs::const_initializer_must_be_a_constant_expression`.
  This clears the two `const` divergences from the spec baseline (44 known,
  67 of 111 pass).
- The conformance runner no longer passes rejected examples vacuously. It now
  strips an inline `// error[E…]` annotation from the staged source, keeps a
  fully-commented statement's trailing annotation as a comment, inserts a
  rejected line at the position left after earlier rejected lines are removed,
  and reads codes only from diagnostic headers instead of echoed snippets. The
  fix cleared 6 cases, exposed 19 masked divergences (now triaged in
  `spec_conformance_baseline.tsv`: 46 known, 65 of 111 pass), and the parser now
  accepts the pinned examples' prefix parameter mode (`fn watch(mut xs: …)`)
  and a brace map's string key (`{ "a": 1 }`). A default integer literal above
  `u64::MAX` is now `E0209`. Regression:
  `crates/xulo-cli/tests/spec_conformance.rs` (three runner tests) and
  `crates/xulo-semantic/tests/semantic_tests.rs::default_integer_literal_above_u64_max_is_e0209`.
- Stage 1's C backend now matches the stage 0 runtime on the last four
  drift-governance deviations (D-01–D-04). Integral `+`/`-`/`*`, `/`, `%`,
  `**`, and unary `-` are checked and fail with `panic: integer overflow`
  instead of wrapping or hitting C UB; runtime failures print stage 0's
  `panic: <message>` prefix; floats keep the plain-decimal shortest form
  (`1e300`, `1e-300`, `1e21`); `u64` annotations and integer literals above
  `i64::MAX` are rejected with `E0005` instead of being mis-tagged as `int`.
  Windows C builds also link `advapi32` explicitly (the runtime's
  `GetUserNameA` call previously depended on the host linker's defaults), and
  the Windows runtime includes `<process.h>` for `_getpid`.
  Regression:
  `crates/xupm/tests/stage1_build.rs::stage1_c_backend_traps_like_the_stage0_runtime`,
  `::stage1_c_backend_matches_stage0_float_formatting`, and
  `::stage1_rejects_u64_in_the_bootstrap_subset`.
- Windows `extern "C"` FFI now works end to end. The interpreter resolves a
  symbol through the process's loaded modules (`GetProcAddress`, falling back to
  the executable and UCRT) instead of returning an "unsupported" error; the JIT
  already resolved through cranelift-jit's Windows lookup, and AOT leaves the
  import to the linker so it binds the UCRT. `examples/ffi/ffi.xulo` no longer
  carries `@no-aot windows` / `@no-run windows`, so the AOT differential covers
  it on every host. Regression: `crates/xulo-runtime/tests/ffi_tests.rs` and
  `crates/xulo-compiler/tests/ffi_codegen_tests.rs` (both un-gated from Unix).
- Both engines now enforce the `f32` format (T12). The interpreter carries
  `Value::Float32`/`Num::Float32` and rounds binding annotations, parameters,
  returns, and every f32 arithmetic result; the AOT backend rounds f32
  bindings, parameters, returns, struct fields, and f32 operations through
  `xulo_round_f32`. `let o: f32 = 0.8` now prints
  `0.800000011920929` on both paths, and `1.0 / 3.0` in f32 prints
  `0.3333333432674408`. `types/float_math.xulo` covers values that differ
  between binary32 and binary64. Regression:
  `crates/xulo-runtime/tests/type_system.rs::f32_rounds_at_binding_and_arithmetic`
  and
  `crates/xulo-compiler/tests/codegen_tests.rs::test_codegen_fixed_width_float_annotations_print_as_floats`.
- AOT: `spawn` / `lock` / service now lower. The compiled concurrency model is
  synchronous: a spawn block runs immediately (so `await` is the identity),
  `lock name { … }` evaluates its block, `spawn.process Type { … }` builds a
  deep-copied service state and calls the compiled `on_message` on `send`, and
  `reply(v)` records the value `send` returns. This matches the interpreter's
  eager async model for the checked forms. `concurrency/spawn_lock.xulo` no
  longer carries `@no-aot`. Regression:
  `crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs::spawn_lock_and_services_follow_the_compiled_model`.
- AOT: dynamic trait dispatch through a generic parameter now works. Struct
  construction registers the source type name alongside the value-category
  marker (and copies preserve it), and `dyn:{Trait}:{method}` picks the
  registered `impl_{Trait}_{Type}_{method}` by testing the receiver's runtime
  struct/enum name (primitive impls use the runtime tag). This covers
  `Area::area(x)` with `x: T`, generic `impl`s such as `Box2<T>`, and inherent
  methods that loop over a `List<T>`. `types/generics_impl.xulo` no longer
  carries `@no-aot`. Regression:
  `crates/xulo-compiler/tests/trait_dispatch_codegen_tests.rs::generic_trait_dispatch_runs`.
- AOT: match guards, or-patterns, numeric range patterns, and type patterns now
  lower instead of being rejected. Guards run after the pattern's bindings; an
  or-pattern tests each alternative before one shared arm body; ranges compare
  numbers (and strings) with the same semantics as the interpreter; type
  patterns test the runtime tag and enum type name. Direct calls now pass the
  runtime tags of dynamic (`unknown`/unannotated/union/optional) arguments, so
  `bool`/`int` patterns distinguish values that share the `0`/`1` slot.
  `patterns/match_patterns.xulo` no longer carries `@no-aot`. Regression:
  `crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs::match_guards_or_ranges_and_type_patterns`.
- AOT: `defer` now lowers to a per-block queue instead of being rejected. The
  operand runs in reverse registration order when its block exits, including
  before `return` and `?` propagation, and on loop `break`/`continue`; nested
  blocks, closures, and match arm blocks drain their own queues.
  `functions/defer.xulo` no longer carries `@no-aot`. Regression:
  `crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs::defer_runs_at_block_exit_in_reverse`
  and `::defer_runs_on_break_continue_and_return`.
- AOT: nested tuple positional access (`nested.0.0`) now resolves the element
  type through every level, and a tuple returned from or passed to a function
  keeps its array tag instead of printing as a raw pointer. `types/tuples.xulo`
  no longer carries `@no-aot`. Regression:
  `crates/xulo-compiler/tests/aot_syntax_coverage_tests.rs::nested_tuple_access_and_tuple_pass_return`.
- AOT: `+` on a string or list bound from an enum payload no longer goes
  through the dynamic-number path. `match std::fs::read(p) { Result::Ok(dir) =>`
  used to hand a float to `xulo_string_concat`, so `dir + "/file"` crashed (or
  concatenated two raw pointers when both operands were payload bindings).
  Regression:
  `crates/xulo-compiler/tests/codegen_tests.rs::test_codegen_string_concat_with_match_payload_binding`
  and `::test_codegen_list_concat_with_match_payload_binding`.
- AOT: user functions now emit as `xulo_fn_<name>`, so a program may define
  `fn read(…)`, `fn listen(…)`, or any other name that libc/Winsock also uses
  without the link failing with `LNK2005: listen already defined`. The entry
  point stays `main` and `pub extern "C" fn` exports keep their declared name.
  Regression: `crates/xulo-compiler/tests/aot_symbols.rs`.
- AOT: `string.split` with a compiled `RegExp` was lowered as a string
  separator, so `"ada, bob".split(re)` split into characters. The intrinsic now
  carries the argument's tag and dispatches to `RegExp::split` for a regex
  payload. Regression:
  `crates/xulo-compiler/tests/regex_codegen_tests.rs::string_split_follows_a_compiled_regexp`
  and `::string_split_accepts_a_separator_string`.

## [0.1.0] - 2026-11-14

First public release, stage 0: the Rust bootstrap toolchain.

### Added

- Native Rust interpreter (`xulo run`) for the core language: literals,
  variables, functions, closures, recursion, control flow, `match`, enums,
  lists, objects, `try`/`catch`, optional chaining, null coalescing, spread,
  `print`/`println`/`str`, `async`/`await`, local `import`/`pub`, and UI
  component syntax.
- CLI (`xulo`) with `run`, `check`, `fmt`, and `repl`.
- `xupm` package manager with native AOT builds through the LLVM 22 backend
  (statically linked LLVM in distribution builds, Cranelift fallback when the
  `llvm` feature is disabled).
- LSP language server (`xulo-analyzer`) with diagnostics, go-to-definition,
  hover, references, outline, formatting, and semantic highlighting.
- Terminal renderer for `main(): View` (`xulo run --render terminal`).
- Language and standard-library specifications in the `spec` and `std-spec`
  submodules (version 0.1.0).
- `install.sh` and a `curl | sh` install path for GitHub Releases, plus
  `install.ps1` for Windows PowerShell.
- Multi-platform release artifacts: Linux x86_64, macOS Intel/Apple Silicon,
  and Windows x86_64.
- MIT license.

### Changed

- Removed the UI examples (`examples/ui.xulo`, `examples/layout.xulo`) and the
  `@xulo/ui` demo shim; the UI language surface and renderers remain available.

### Known limitations

- The Linux release builds `xupm` with statically linked LLVM 22. The macOS
  and Windows stage-0 previews build `xupm` with the Cranelift backend instead
  (no LLVM installation required); install LLVM 22 and build with
  `--features distribution` for the product AOT path on those hosts.
- The interpreter's FFI (`extern "C"`) lookup is Unix-only; Windows returns a
  clear "unsupported" error. AOT FFI is wired for the Windows calling
  convention but is not yet covered by CI.
- The webview and Skia renderers are not part of this release artifact; the
  webview backend also requires the `wasm32-unknown-unknown` target at build
  time.
- The formatter discards comments.
- `@Environment` is not implemented.
- The compiler is a tree-walking interpreter plus an LLVM/Cranelift AOT path;
  self-hosting is not part of stage 0.

[0.1.0]: https://github.com/xulo-lang/xulo/releases/tag/v0.1.0
