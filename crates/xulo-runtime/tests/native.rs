use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use xulo_core::error::{ErrorKind, XuloError};
use xulo_lexer::tokenize;
use xulo_parser::parse_program;
use xulo_runtime::interpreter::{Interpreter, ModuleExports, RunError};
use xulo_runtime::value::Value;

/// Full pipeline: lex -> parse -> semantic check -> run the native interpreter.
fn run(src: &str) -> Result<Vec<String>, XuloError> {
    let tokens = tokenize(src).unwrap();
    let program = parse_program(&tokens).unwrap();
    xulo_semantic::analyze(&program)?;
    let interp = Interpreter::new();
    interp.run(&program)
}

/// Like [`run`], but applies trait-dispatch annotations so `Trait::method`
/// calls resolve to their mangled `impl_{Trait}_{Type}_{method}` function.
fn run_annotated(src: &str) -> Result<Vec<String>, XuloError> {
    let tokens = tokenize(src).unwrap();
    let mut program = parse_program(&tokens).unwrap();
    let result = xulo_semantic::analyze_with(&program, &[], &[], &[])?;
    xulo_semantic::apply_trait_dispatch(&mut program, &result.trait_dispatch);
    let interp = Interpreter::new();
    interp.run(&program)
}

/// Lex + parse only (no semantic check), so interpreter-level guards (e.g.
/// "undefined variable") can be exercised without the type checker rejecting
/// the program first.
fn run_raw(src: &str) -> Result<Vec<String>, XuloError> {
    let tokens = tokenize(src).unwrap();
    let program = parse_program(&tokens).unwrap();
    let interp = Interpreter::new();
    interp.run(&program)
}

fn run_ok(src: &str) -> Vec<String> {
    run(src).unwrap_or_else(|e| panic!("{}", e.message))
}

/// Execute a library module and then an entry module that `import`s from it,
/// wiring runtime values just as the CLI does: one shared interpreter, so
/// top-level `print`s from every module accumulate in order. Files are lex +
/// parse only — the interpreter (not the type checker) is under test.
fn exec_module_pair(
    lib_src: &str,
    entry_src: &str,
    resolve: impl FnOnce(ModuleExports) -> Vec<(String, Value)>,
) -> Result<Vec<String>, XuloError> {
    fn run_module(
        interp: &Interpreter,
        src: &str,
        imports: &[(String, Value)],
        run_main: bool,
    ) -> Result<ModuleExports, XuloError> {
        let tokens = tokenize(src).unwrap();
        let program = parse_program(&tokens).unwrap();
        interp
            .exec_module(&program, imports, run_main)
            .map_err(|e| match e {
                RunError::Err(e) => e,
                RunError::Throw(v) => XuloError::new(
                    ErrorKind::Runtime,
                    format!("uncaught exception: {}", v.format()),
                ),
            })
    }

    let interp = Interpreter::new();
    let exports = run_module(&interp, lib_src, &[], false)?;
    let imports = resolve(exports);
    run_module(&interp, entry_src, &imports, true)?;
    Ok(interp.take_output())
}

/// Pick the named exports out of a module's bindings, mirroring `import { a, b }`.
fn pick(exports: ModuleExports, names: &[&str]) -> Vec<(String, Value)> {
    let ModuleExports { bindings, .. } = exports;
    let mut by_name: HashMap<String, Value> = bindings.into_iter().collect();
    names
        .iter()
        .map(|name| {
            let value = by_name
                .remove(*name)
                .unwrap_or_else(|| panic!("library has no export named `{name}`"));
            (name.to_string(), value)
        })
        .collect()
}

#[test]
fn trait_dispatch_runs_mangled_impl() {
    let out = run_annotated(
        r#"
        trait Area { fn area(self): number }
        type Rectangle = object
        impl Area for Rectangle {
            fn area(self): number { return self.w * self.h }
        }
        fn rect(w: number, h: number): Rectangle {
            let r = { w: w, h: h }
            r
        }
        fn main() { print(Area::area(rect(3, 4))) }
        "#,
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    assert_eq!(out, vec!["12".to_string()]);
}

#[test]
fn arithmetic() {
    assert_eq!(
        run_ok(
            r#"
            fn main() {
                print(10 + 3)
                print(10 - 3)
                print(10 * 3)
                print(10 / 3)
                print(1 + 2 * 3)
            }
            "#
        ),
        vec!["13", "7", "30", "3.3333333333333335", "7"]
    );
}

#[test]
fn string_concat_and_print() {
    assert_eq!(
        run_ok(r#"fn main() { print("Hello, " + "world!") }"#),
        vec!["Hello, world!"]
    );
}

#[test]
fn hello_example() {
    assert_eq!(
        run_ok(include_str!("../../../examples/hello.xulo")),
        vec!["Hello, world!"]
    );
}

#[test]
fn if_else_example() {
    assert_eq!(
        run_ok(include_str!("../../../examples/if_else.xulo")),
        vec!["7", "two is not greater than three"]
    );
}

#[test]
fn fibonacci_example() {
    assert_eq!(
        run_ok(include_str!("../../../examples/fibonacci.xulo")),
        vec!["0", "1", "1", "2", "3", "5", "8", "13", "21", "34"]
    );
}

#[test]
fn recursion_with_if_expression() {
    let out = run_ok(
        r#"
        fn fact(n: number): number {
            if n <= 1 { return 1 } else { return n * fact(n - 1) }
        }
        fn main() { print(fact(6)) }
        "#,
    );
    assert_eq!(out, vec!["720"]);
}

#[test]
fn if_expression_prefix_return_short_circuits_like_js() {
    // A `return` before a value-position `if` arm's trailing expression is the
    // arm's value (the JS codegen's IIFE exits on it); the native runtime used
    // to report "unexpected return" and diverge.
    let out = run_ok(
        r#"
        fn main(): number {
            let x = if true { return 5
                3 } else { 2 }
            print(str(x))
            x
        }
        "#,
    );
    assert_eq!(out, vec!["5"]);
}

#[test]
fn unbounded_recursion_errors_instead_of_crashing() {
    // Unbounded recursion used to overflow the stack and abort the process;
    // it must surface as a clean runtime error past the depth limit. Runs on
    // a dedicated big-stack thread so the test harness's default (2 MiB)
    // test-thread stack can never overflow before the interpreter's own depth
    // guard fires — the guard is what's under test, not the host stack.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            // Unbounded *sync* recursion errors cleanly.
            let err = run_raw(
                r#"
                fn rec(n: number): number { rec(n - 1) }
                fn main() { print(str(rec(1))) }
                "#,
            )
            .unwrap_err();
            assert!(
                err.message.contains("call depth exceeded"),
                "got: {}",
                err.message
            );

            // Async recursion hits the same limit before exhausting memory.
            let err = run_raw(
                r#"
                fn rec(n: number): async number { await rec(n - 1) }
                fn main(): async { print(str(await rec(1))) }
                "#,
            )
            .unwrap_err();
            assert!(
                err.message.contains("call depth exceeded"),
                "got: {}",
                err.message
            );

            // Recursion within the limit keeps working (sync and async).
            assert_eq!(
                run_ok(
                    r#"
                    fn sum(n: number): number { if n <= 0 { return 0 } sum(n - 1) + n }
                    fn main() { print(str(sum(100))) }
                    "#,
                ),
                vec!["5050"]
            );
            assert_eq!(
                run_ok(
                    r#"
                    fn step(n: number): async number { if n <= 0 { return 0 } await step(n - 1) n }
                    fn main(): async { print(str(await step(100))) }
                    "#,
                ),
                vec!["100"]
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn implicit_return() {
    let out = run_ok(
        r#"
        fn double(x: number): number { x * 2 }
        fn main() { print(double(21)) }
        "#,
    );
    assert_eq!(out, vec!["42"]);
}

#[test]
fn anonymous_function_closure() {
    let out = run_ok(
        r#"
        fn main() {
            let base = 100
            let add = fn(x: number): number { x + base }
            print(add(5))
        }
        "#,
    );
    assert_eq!(out, vec!["105"]);
}

#[test]
fn higher_order_function() {
    let out = run_ok(
        r#"
        fn apply(f: fn(number): number, x: number): number { f(x) }
        fn main() {
            print(apply(fn(n: number): number { n * n }, 7))
        }
        "#,
    );
    assert_eq!(out, vec!["49"]);
}

#[test]
fn named_arguments_are_reordered() {
    let out = run_ok(
        r#"
        fn sub(a: number, b: number): number { a - b }
        fn main() { print(sub(b: 3, a: 10)) }
        "#,
    );
    assert_eq!(out, vec!["7"]);
}

#[test]
fn default_and_optional_params() {
    let out = run_ok(
        r#"
        fn greet(name: string, punctuation: string = "!"): string { "Hello, " + name + punctuation }
        fn maybe(x: number?): number { if x == null { 0 } else { x } }
        fn main() {
            print(greet("lyy"))
            print(greet(name: "lyy", punctuation: "?"))
            print(maybe(null))
            print(maybe(42))
        }
        "#,
    );
    assert_eq!(out, vec!["Hello, lyy!", "Hello, lyy?", "0", "42"]);
}

#[test]
fn if_as_expression() {
    let out = run_ok(
        r#"
        fn main() {
            let value = if true { 1 } else { 2 }
            print(value)
            print(if false { "yes" } else { "no" })
        }
        "#,
    );
    assert_eq!(out, vec!["1", "no"]);
}

#[test]
fn ternary() {
    assert_eq!(
        run_ok(r#"fn main() { print(2 > 3 ? "big" : "small") }"#),
        vec!["small"]
    );
}

#[test]
fn boolean_logic() {
    assert_eq!(
        run_ok(
            r#"
            fn main() {
                print(true and false)
                print(true or false)
                print(!true)
                print(5 > 3 and 2 < 4)
            }
            "#
        ),
        vec!["false", "true", "false", "true"]
    );
}

#[test]
fn for_over_list() {
    let out = run_ok(
        r#"
        fn main() {
            let total = 0
            for x in [1, 2, 3, 4] {
                total = total + x
            }
            print(total)
        }
        "#,
    );
    assert_eq!(out, vec!["10"]);
}

#[test]
fn for_over_list_iterates_live_like_js() {
    // Mutations inside the loop body are visible to later iterations, exactly
    // like JS `for...of` (a snapshot used to diverge: `xs[1] = 99` inside the
    // body still yielded the old value here but the new one in JS).
    let out = run_ok(
        r#"
        fn main() {
            let xs = [1, 2]
            for x in xs {
                print(str(x))
                xs[1] = 99
            }
        }
        "#,
    );
    assert_eq!(out, vec!["1", "99"]);
}

#[test]
fn for_over_range() {
    let out = run_ok(
        r#"
        fn main() {
            for i in 0..<3 {
                print(i)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["0", "1", "2"]);
}

#[test]
fn while_loop() {
    let out = run_ok(
        r#"
        fn main() {
            let i = 0
            while i < 4 {
                print(i)
                i = i + 1
            }
        }
        "#,
    );
    assert_eq!(out, vec!["0", "1", "2", "3"]);
}

#[test]
fn enum_with_payload() {
    let out = run_ok(
        r#"
        enum Result { Success(number) Failure }
        fn main() {
            let r = Result::Success(42)
            print(r)
            print(match r {
                Result::Success(n) => "ok: " + str(n)
                Result::Failure => "fail"
                _ => "?" 
            })
        }
        "#,
    );
    assert_eq!(out, vec!["Result.Success(42)", "ok: 42"]);
}

#[test]
fn enum_multi_payload_binds_list() {
    let out = run_ok(
        r#"
        enum Shape { Point(number, number) }
        fn main() {
            let p = Shape::Point(3, 4)
            print(match p {
                Shape::Point(x, y) => "(" + str(x) + "," + str(y) + ")"
            })
        }
        "#,
    );
    assert_eq!(out, vec!["(3,4)"]);
}

#[test]
fn match_on_plain_enum_and_literal() {
    let out = run_ok(
        r#"
        enum Color { Red Green Blue }
        fn main() {
            let c = Color::Green
            print(match c {
                Color::Red => "r"
                Color::Green => "g"
                Color::Blue => "b"
            })
            print(match 3 {
                1 => "one"
                3 => "three"
                _ => "other"
            })
            print(match "ab" {
                "ab" => "matched string"
                _ => "no"
            })
        }
        "#,
    );
    assert_eq!(out, vec!["g", "three", "matched string"]);
}

#[test]
fn list_spread() {
    let out = run_ok(
        r#"
        fn main() {
            let xs = [1, 2]
            print([0, ...xs, 3])
        }
        "#,
    );
    assert_eq!(out, vec!["[0, 1, 2, 3]"]);
}

#[test]
fn list_concat_with_plus() {
    let out = run_ok(r#"fn main() { print([1, 2] + [3, 4]) }"#);
    assert_eq!(out, vec!["[1, 2, 3, 4]"]);
}

#[test]
fn list_index_validates_integer_indices() {
    // Fractional / negative / NaN indices must error, not silently read the
    // wrong element (Rust's `f64 as usize` truncates and saturates).
    let err = run(r#"fn main() { let xs = [10, 20, 30] print(xs[1.5]) }"#).unwrap_err();
    assert!(
        err.message.contains("non-negative integer"),
        "got: {}",
        err.message
    );

    let err = run(r#"fn main() { let xs = [10, 20, 30] print(xs[-1]) }"#).unwrap_err();
    assert!(
        err.message.contains("non-negative integer"),
        "got: {}",
        err.message
    );

    // Assignments go through the same validation.
    let err = run(r#"fn main() { let xs = [10, 20, 30] xs[0.5] = 1 print(xs) }"#).unwrap_err();
    assert!(
        err.message.contains("non-negative integer"),
        "got: {}",
        err.message
    );

    // Valid indices still work.
    let out = run_ok(r#"fn main() { let xs = [10, 20, 30] print(xs[2]) }"#);
    assert_eq!(out, vec!["30"]);
}

#[test]
fn string_index_reads_characters() {
    let out = run_ok(r#"fn main() { let s = "abc" print(s[1]) }"#);
    assert_eq!(out, vec!["b"]);
    let err = run(r#"fn main() { let s = "abc" print(s[3]) }"#).unwrap_err();
    assert!(
        err.message.contains("out of bounds"),
        "got: {}",
        err.message
    );
}

#[test]
fn string_length_counts_utf16_units() {
    // JS `.length` counts UTF-16 code units: "😀" is a surrogate pair.
    // (`run_raw`: the semantic phase currently types `string.length` away, so
    // the interpreter-level behavior is exercised directly.)
    let out = run_raw(r#"fn main() { let s = "😀" print(str(s.length)) }"#).unwrap();
    assert_eq!(out, vec!["2"]);
    let out = run_raw(r#"fn main() { let s = "abc" print(str(s.length)) }"#).unwrap();
    assert_eq!(out, vec!["3"]);
}

#[test]
fn default_parameter_evaluates_in_callee_scope_across_modules() {
    // A default expression referencing a module-level name must resolve in the
    // *defining* module's scope (like the JS closure), not the caller's — a
    // caller-local `rate` must not leak in (and must not be required).
    let out = exec_module_pair(
        r#"
        let rate = 100
        export fn make(x: number, y: number = rate): number { y }
        "#,
        r#"
        import { make } from "./lib"
        fn main() {
            let rate = 999
            print(str(make(5)))
            print(str(make(5, 7)))
        }
        "#,
        |exports| pick(exports, &["make"]),
    )
    .unwrap();
    assert_eq!(out, vec!["100", "7"]);
}

#[test]
fn top_level_async_without_main_is_driven_to_completion() {
    // A script with no `main` that spawns async work must still drain the task
    // queue (the JS path drains microtasks); used to park forever.
    let out = run_ok(
        r#"
        fn pause(): async { }
        fn work(): async number {
            await pause()
            42
        }
        fn done(): async {
            let v = await work()
            print(str(v))
        }
        let d = done()
        "#,
    );
    assert_eq!(out, vec!["42"]);
}

#[test]
fn cyclic_values_print_without_overflowing() {
    // A self-referencing object must render a cycle marker instead of
    // recursing forever (this used to overflow the stack and abort).
    let out = run_ok(
        r#"
        fn main() {
            let o: object = {}
            o.x = o
            print(o)
        }
        "#,
    );
    assert_eq!(out, vec!["{ x: { <cycle> } }"]);

    // A self-referencing list (`run_raw`: the type checker rejects storing a
    // list into a `number` slot, so the interpreter-level cycle handling is
    // exercised directly).
    let out = run_raw(
        r#"
        fn main() {
            let xs = [1]
            xs[0] = xs
            print(xs)
        }
        "#,
    )
    .unwrap();
    assert_eq!(out, vec!["[[<cycle>]]"]);

    // A shared (non-cyclic) reference is still rendered twice.
    let out = run_ok(
        r#"
        fn main() {
            let a = [1]
            print([a, a])
        }
        "#,
    );
    assert_eq!(out, vec!["[[1], [1]]"]);
}

#[test]
fn object_literal_member_access_and_assign() {
    let out = run_ok(
        r#"
        fn main() {
            let person = { name: "lyy", age: 30 }
            print(person.name)
            person.age = 31
            print(person.age)
            print(person)
        }
        "#,
    );
    assert_eq!(out, vec!["lyy", "31", "{ name: lyy, age: 31 }"]);
}

#[test]
fn object_spread() {
    let out = run_ok(
        r#"
        fn main() {
            let base = { a: 1 }
            print({ ...base, b: 2 })
        }
        "#,
    );
    assert_eq!(out, vec!["{ a: 1, b: 2 }"]);
}

#[test]
fn index_read_write() {
    let out = run_ok(
        r#"
        fn main() {
            let xs = [10, 20, 30]
            print(xs[1])
            xs[1] = 99
            print(xs)
            let dict = { k: "v" }
            print(dict["k"])
        }
        "#,
    );
    assert_eq!(out, vec!["20", "[10, 99, 30]", "v"]);
}

#[test]
fn list_alias_is_shared_like_js() {
    let out = run_ok(
        r#"
        fn main() {
            let a = [1, 2]
            let b = a
            b[0] = 9
            print(a)
        }
        "#,
    );
    assert_eq!(out, vec!["[9, 2]"]);
}

#[test]
fn nullish_and_optional_access() {
    let out = run_ok(
        r#"
        fn main() {
            let x = null
            print(x ?? "fallback")
            print(null?.field)
            let obj = { a: 5 }
            print(obj?.a)
        }
        "#,
    );
    assert_eq!(out, vec!["fallback", "null", "5"]);
}

#[test]
fn try_catch_throw() {
    let out = run_ok(
        r#"
        fn main() {
            try {
                throw "boom"
            } catch (e) {
                print("caught " + e)
            }
            print("after")
        }
        "#,
    );
    assert_eq!(out, vec!["caught boom", "after"]);
}

#[test]
fn try_catch_reraises_other_errors() {
    let err = run_raw(
        r#"
        fn main() {
            try {
                throw "boom"
            } catch (e) {
                print(e)
            }
            throw "outside"
        }
        "#,
    )
    .unwrap_err();
    assert!(
        err.message.contains("uncaught exception: outside"),
        "{}",
        err.message
    );
}

#[test]
fn call_function_value_from_member() {
    let out = run_ok(
        r#"
        fn main() {
            let counter = { inc: fn(x: number): number { x + 1 } }
            print(counter.inc(5))
        }
        "#,
    );
    assert_eq!(out, vec!["6"]);
}

#[test]
fn method_call_on_object() {
    let out = run_ok(
        r#"
        fn main() {
            let math = { double: fn(x: number): number { x * 2 } }
            print(math.double(21))
        }
        "#,
    );
    assert_eq!(out, vec!["42"]);
}

#[test]
fn str_builtin() {
    assert_eq!(
        run_ok(r#"fn main() { print(str(3.14)) print(str([1, 2])) }"#),
        vec!["3.14", "[1, 2]"]
    );
}

#[test]
fn print_multiple_args() {
    assert_eq!(
        run_ok(r#"fn main() { print("a", 1, true) }"#),
        vec!["a 1 true"]
    );
}

#[test]
fn rejects_ui_components() {
    let err = run(r#"
        fn main(): Component {
            VStack { Text("hi") }
        }
        "#)
    .unwrap_err();
    assert!(err.message.contains("UI components"), "{}", err.message);
}

#[test]
fn await_outside_async_rejected() {
    // The semantic layer normally forbids this; exercise the interpreter's
    // guard directly (parse only).
    let err = run_raw(r#"fn main() { print(await 5) }"#).unwrap_err();
    assert!(
        err.message
            .contains("`await` may only be used inside an `async`"),
        "{}",
        err.message
    );
}

#[test]
fn rejects_imports() {
    let err = run(r#"import { foo } from "bar" fn main() { print(1) }"#).unwrap_err();
    assert!(err.message.contains("imports"), "{}", err.message);
}

#[test]
fn rejects_state_decorators() {
    // The semantic layer forbids `@State` outside a `Component` function, so
    // this exercises the interpreter's guard directly (parse only).
    let err = run_raw(r#"@State let count: number = 0 fn main() { print(1) }"#).unwrap_err();
    assert!(err.message.contains("reactive state"), "{}", err.message);
}

#[test]
fn rejects_dollar_binding() {
    let err = run_raw(r#"fn main() { print($x) }"#).unwrap_err();
    assert!(err.message.contains("not supported"), "{}", err.message);
}

#[test]
fn undefined_function_at_runtime() {
    let err = run_raw(r#"fn main() { print(nope(1)) }"#).unwrap_err();
    assert!(
        err.message.contains("undefined function `nope`"),
        "{}",
        err.message
    );
}

#[test]
fn undefined_variable_at_runtime() {
    let err = run_raw(r#"fn main() { print(nope) }"#).unwrap_err();
    assert!(
        err.message.contains("undefined variable `nope`"),
        "{}",
        err.message
    );
}

#[test]
fn non_exhaustive_match_errors() {
    let err = run_raw(r#"fn main() { print(match 5 { 1 => "one" }) }"#).unwrap_err();
    assert!(
        err.message.contains("non-exhaustive match"),
        "{}",
        err.message
    );
}

#[test]
fn type_annotation_is_erased_at_runtime() {
    // The semantic layer accepts these; the interpreter runs them.
    let out = run_ok(
        r#"
        type Pair = { x: number, y: number }
        fn main() {
            let p: Pair = { x: 1, y: 2 }
            print(p.x + p.y)
        }
        "#,
    );
    assert_eq!(out, vec!["3"]);
}

#[test]
fn export_fn_declares_and_runs_main() {
    let out = run_ok(
        r#"
        pub fn add(a: number, b: number): number { a + b }
        pub fn main() { print(add(2, 3)) }
        "#,
    );
    assert_eq!(out, vec!["5"]);
}

#[test]
fn returns_null_from_plain_functions() {
    let out = run_ok(
        r#"
        fn nothing() { }
        fn main() { print(nothing()) }
        "#,
    );
    assert_eq!(out, vec!["null"]);
}

#[test]
fn async_basic_await() {
    let out = run_ok(
        r#"
        fn half(): async number { 2 }
        fn main(): async {
            let a = (await half()) + 1
            let b = (await half()) * 3
            print(a)
            print(b)
        }
        "#,
    );
    assert_eq!(out, vec!["3", "6"]);
}

#[test]
fn async_coroutines_interleave_in_fifo_order() {
    let out = run_ok(
        r#"
        fn pause(): async { }
        fn second(): async {
            print("b1")
            await pause()
            print("b2")
        }
        fn first(): async {
            print("a1")
            await second()
            print("a2")
        }
        fn third(): async {
            print("c1")
            await pause()
        }
        fn main(): async {
            let p1 = first()
            let p3 = third()
            await p1
            await p3
        }
        "#,
    );
    assert_eq!(out, vec!["a1", "b1", "c1", "b2", "a2"]);
}

#[test]
fn async_await_on_settled_promise_defers() {
    let out = run_ok(
        r#"
        fn helper(): async {
            print("inner")
            return 1
        }
        fn main(): async {
            let p = helper()
            print("after-call")
            let v = await p
            print("after-await")
            print(v)
        }
        "#,
    );
    assert_eq!(out, vec!["inner", "after-call", "after-await", "1"]);
}

#[test]
fn async_try_catch_rejection() {
    let out = run_ok(
        r#"
        fn boom(): async { throw "boom" }
        fn main(): async {
            try {
                await boom()
            } catch (e) {
                print("caught " + e)
            }
            print("after")
        }
        "#,
    );
    assert_eq!(out, vec!["caught boom", "after"]);
}

#[test]
fn async_recursion_statement_if_propagates_return() {
    let out = run_ok(
        r#"
        fn sum(n: number): async number {
            if n == 0 {
                return 0
            }
            return n + await sum(n - 1)
        }
        fn main(): async { print(await sum(5)) }
        "#,
    );
    assert_eq!(out, vec!["15"]);
}

#[test]
fn async_function_value_return_type() {
    let out = run_ok(
        r#"
        fn main(): async {
            let f = fn(x: number): async number { x * 2 }
            print(await f(21))
        }
        "#,
    );
    assert_eq!(out, vec!["42"]);
}

#[test]
fn async_main_rejection_is_uncaught() {
    let err = run_raw(r#"fn main(): async { throw "kaboom" }"#).unwrap_err();
    assert!(
        err.message.contains("uncaught exception: kaboom"),
        "{}",
        err.message
    );
}

#[test]
fn sync_main_calls_async_prefix_runs_inline() {
    let out = run_ok(
        r#"
        fn pause(): async { }
        fn late(): async {
            print("late")
            await pause()
        }
        fn main() {
            late()
            print("sync")
        }
        "#,
    );
    assert_eq!(out, vec!["late", "sync"]);
}

#[test]
fn sync_function_recursion_statement_if_propagates_return() {
    let out = run_ok(
        r#"
        fn sum(n: number): number {
            if n == 0 {
                return 0
            }
            return n + sum(n - 1)
        }
        fn main() { print(sum(5)) }
        "#,
    );
    assert_eq!(out, vec!["15"]);
}

#[test]
fn module_named_imports_bind_fn_and_const() {
    let out = exec_module_pair(
        r#"
        export fn add(a: number, b: number): number { a + b }
        export const PI = 3.14
        "#,
        r#"
        import { add, PI } from "lib"
        fn main() {
            print(add(2, 3))
            print(PI)
        }
        "#,
        |exports| pick(exports, &["add", "PI"]),
    )
    .unwrap();
    assert_eq!(out, vec!["5", "3.14"]);
}

#[test]
fn module_import_alias() {
    let out = exec_module_pair(
        r#"
        export fn add(a: number, b: number): number { a + b }
        "#,
        r#"
        import { add as plus } from "lib"
        fn main() { print(plus(1, 1)) }
        "#,
        |exports| {
            pick(exports, &["add"])
                .into_iter()
                .map(|(_, v)| ("plus".to_string(), v))
                .collect()
        },
    )
    .unwrap();
    assert_eq!(out, vec!["2"]);
}

#[test]
fn module_namespace_import_exposes_named_fields() {
    let out = exec_module_pair(
        r#"
        export const HOURS_IN_DAY = 24
        export fn hours(): number { HOURS_IN_DAY }
        "#,
        r#"
        import * as cal from "lib"
        fn main() {
            print(cal.HOURS_IN_DAY)
            print(cal.hours())
        }
        "#,
        |exports| {
            let fields: Vec<(String, Value)> = exports
                .bindings
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            vec![(
                "cal".to_string(),
                Value::Object(Rc::new(RefCell::new(fields))),
            )]
        },
    )
    .unwrap();
    assert_eq!(out, vec!["24", "24"]);
}

#[test]
fn module_default_export_importable() {
    let out = exec_module_pair(
        r#"
        export default fn greet(): string { "hi" }
        "#,
        r#"
        import g from "lib"
        fn main() { print(g()) }
        "#,
        |exports| vec![("g".to_string(), exports.default.clone().unwrap())],
    )
    .unwrap();
    assert_eq!(out, vec!["hi"]);
}

#[test]
fn module_library_side_effects_run_before_entry() {
    let out = exec_module_pair(
        r#"
        print("lib-init")
        export fn tag(): string { "lib" }
        "#,
        r#"
        import { tag } from "lib"
        fn main() { print(tag()) }
        "#,
        |exports| pick(exports, &["tag"]),
    )
    .unwrap();
    assert_eq!(out, vec!["lib-init", "lib"]);
}

#[test]
fn exec_module_default_export_of_plain_main_fn_is_importable() {
    // A library whose (non-`main`) default is a plain function still exposes it
    // as the module default.
    let out = exec_module_pair(
        r#"
        export default fn answer(): number { 42 }
        "#,
        r#"
        import a from "lib"
        fn main() { print(a()) }
        "#,
        |exports| vec![("a".to_string(), exports.default.clone().unwrap())],
    )
    .unwrap();
    assert_eq!(out, vec!["42"]);
}
