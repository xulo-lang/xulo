use std::path::Path;
use xulo_compiler::{codegen::CodeGen, compile_to_ir};

fn compile_and_run(source: &str) -> Vec<String> {
    let ir = compile_to_ir(source, Path::new("test.xulo")).expect("IR generation failed");
    let mut codegen = CodeGen::new();
    let code_ptr = codegen.compile(&ir).expect("codegen failed");

    unsafe {
        let func: fn() = std::mem::transmute(code_ptr);
        func();
    }
    xulo_runtime::runtime::xulo_take_output()
}

// -- smoke tests (crash-free only) --

#[test]
fn test_codegen_hello_world() {
    compile_and_run(r#"fn main() { print("Hello from JIT") }"#);
}

#[test]
fn test_codegen_empty_main() {
    compile_and_run("fn main() {}");
}

#[test]
fn test_codegen_let_binding() {
    compile_and_run(r#"fn main() { let x = 42 }"#);
}

#[test]
fn test_codegen_arithmetic() {
    compile_and_run(r#"fn main() { let x = 1 + 2 }"#);
}

#[test]
fn test_codegen_negative_numbers() {
    compile_and_run(r#"fn main() { let x = -5 }"#);
}

#[test]
fn test_codegen_nested_expressions() {
    compile_and_run(
        r#"
fn main() {
    let x = (1 + 2) * 3
    let y = 10 - (2 + 3)
}
"#,
    );
}

#[test]
fn test_codegen_array_literal() {
    compile_and_run(
        r#"
fn main() {
    let arr = [1, 2, 3]
}
"#,
    );
}

#[test]
fn test_codegen_object_literal() {
    compile_and_run(
        r#"
fn main() {
    let obj = { name: "test", value: 42 }
}
"#,
    );
}

#[test]
fn test_codegen_closure() {
    compile_and_run(
        r#"
fn main() {
    let add = fn(a, b) { return a + b }
}
"#,
    );
}

#[test]
fn test_codegen_list_push() {
    compile_and_run(
        r#"
fn main() {
    let arr = [1, 2, 3]
    arr.push(4)
}
"#,
    );
}

// -- output assertion tests --

#[test]
fn test_codegen_print_string() {
    let out = compile_and_run(r#"fn main() { print("hello") }"#);
    assert_eq!(out, vec!["hello"]);
}

#[test]
fn test_codegen_print_int() {
    let out = compile_and_run(r#"fn main() { print(42) }"#);
    assert_eq!(out, vec!["42"]);
}

#[test]
fn test_codegen_print_variable() {
    let out = compile_and_run(
        r#"
fn main() {
    let x = 100
    print(x)
}
"#,
    );
    assert_eq!(out, vec!["100"]);
}

#[test]
fn test_codegen_multiple_prints() {
    let out = compile_and_run(
        r#"
fn main() {
    print("first")
    print("second")
}
"#,
    );
    assert_eq!(out, vec!["first", "second"]);
}

#[test]
fn test_codegen_print_bool() {
    let out = compile_and_run(r#"fn main() { print(true) }"#);
    assert_eq!(out, vec!["true"]);
}

#[test]
fn test_codegen_print_null() {
    let out = compile_and_run(r#"fn main() { print(null) }"#);
    assert_eq!(out, vec!["null"]);
}

#[test]
fn test_codegen_print_array() {
    let out = compile_and_run(r#"fn main() { print([1, 2, 3]) }"#);
    assert_eq!(out, vec!["[1, 2, 3]"]);
}

#[test]
fn test_codegen_print_object() {
    let out = compile_and_run(r#"fn main() { print({a: 1, b: 2}) }"#);
    // 与解释器 Value::format 一致：{ a: 1, b: 2 }
    assert_eq!(out, vec!["{ a: 1, b: 2 }"]);
}

#[test]
fn test_codegen_subtraction() {
    let out = compile_and_run(r#"fn main() { print(10 - 3) }"#);
    assert_eq!(out, vec!["7"]);
}

#[test]
fn test_codegen_multiplication() {
    let out = compile_and_run(r#"fn main() { print(4 * 5) }"#);
    assert_eq!(out, vec!["20"]);
}

#[test]
fn test_codegen_division() {
    let out = compile_and_run(r#"fn main() { print(20 / 4) }"#);
    assert_eq!(out, vec!["5"]);
}

#[test]
fn test_codegen_float_division_not_truncated() {
    // A literal with a decimal point is a float even when integral-valued
    // (`1000000.0`), so division must be floating point, not `sdiv`.
    let out = compile_and_run(
        r#"fn main() {
            print(1.0 / 3.0)
            print(14700 / 1000000.0)
            print(7.0 / 2.0)
        }"#,
    );
    assert_eq!(out, vec!["0.3333333333333333", "0.0147", "3.5"]);
}

// -- array concat --

#[test]
fn test_codegen_array_concat() {
    let out = compile_and_run(r#"fn main() { print([1, 2] + [3, 4]) }"#);
    assert_eq!(out, vec!["[1, 2, 3, 4]"]);
}

#[test]
fn test_codegen_array_concat_strings() {
    let out = compile_and_run(r#"fn main() { print(["a"] + ["b"]) }"#);
    assert_eq!(out, vec!["[a, b]"]);
}

#[test]
fn test_codegen_array_concat_empty_left() {
    let out = compile_and_run(r#"fn main() { print([] + [1]) }"#);
    assert_eq!(out, vec!["[1]"]);
}

#[test]
fn test_codegen_array_concat_empty_right() {
    let out = compile_and_run(r#"fn main() { print([1] + []) }"#);
    assert_eq!(out, vec!["[1]"]);
}

#[test]
fn test_codegen_array_concat_variable() {
    let out = compile_and_run(
        r#"
fn main() {
    let a = [1, 2]
    let b = [3, 4]
    print(a + b)
}
"#,
    );
    assert_eq!(out, vec!["[1, 2, 3, 4]"]);
}

// -- for loops --

#[test]
fn test_codegen_for_loop_integers() {
    let out = compile_and_run(
        r#"
fn main() {
    for i in [10, 20, 30] {
        print(i)
    }
}
"#,
    );
    assert_eq!(out, vec!["10", "20", "30"]);
}

#[test]
fn test_codegen_for_loop_strings() {
    let out = compile_and_run(
        r#"
fn main() {
    for x in ["a", "b", "c"] {
        print(x)
    }
}
"#,
    );
    assert_eq!(out, vec!["a", "b", "c"]);
}

#[test]
fn test_codegen_for_loop_concat() {
    let out = compile_and_run(
        r#"
fn main() {
    let mut out = []
    for x in [1, 2, 3] {
        out = out + [x]
    }
    print(out)
}
"#,
    );
    assert_eq!(out, vec!["[1, 2, 3]"]);
}

#[test]
fn test_codegen_for_loop_concat_strings() {
    let out = compile_and_run(
        r#"
fn main() {
    let mut out = []
    for x in ["a", "b"] {
        out = out + [x]
    }
    print(out)
}
"#,
    );
    assert_eq!(out, vec!["[a, b]"]);
}

// -- generic functions --

#[test]
fn test_codegen_generic_identity_int() {
    let out = compile_and_run(
        r#"
fn identity<T>(value: T): T {
    value
}
fn main() {
    print(identity(42))
}
"#,
    );
    assert_eq!(out, vec!["42"]);
}

#[test]
fn test_codegen_generic_identity_string() {
    let out = compile_and_run(
        r#"
fn identity<T>(value: T): T {
    value
}
fn main() {
    print(identity("hello"))
}
"#,
    );
    assert_eq!(out, vec!["hello"]);
}

#[test]
fn test_codegen_generic_identity_bool() {
    let out = compile_and_run(
        r#"
fn identity<T>(value: T): T {
    value
}
fn main() {
    print(identity(true))
}
"#,
    );
    // 调用点从实参反推 `T = bool`，返回槽按 tag=5 打印。
    assert_eq!(out, vec!["true"]);
}

#[test]
fn test_codegen_generic_twice_int() {
    let out = compile_and_run(
        r#"
fn twice<T>(value: T): List<T> {
    [value, value]
}
fn main() {
    print(twice(7))
}
"#,
    );
    assert_eq!(out, vec!["[7, 7]"]);
}

#[test]
fn test_codegen_generic_twice_string() {
    let out = compile_and_run(
        r#"
fn twice<T>(value: T): List<T> {
    [value, value]
}
fn main() {
    print(twice("x"))
}
"#,
    );
    assert_eq!(out, vec!["[x, x]"]);
}

#[test]
fn test_codegen_generic_first() {
    let out = compile_and_run(
        r#"
fn first<T>(list: List<T>): T {
    list[0]
}
fn main() {
    print(first([10, 20, 30]))
}
"#,
    );
    assert_eq!(out, vec!["10"]);
}

#[test]
fn test_codegen_generic_first_string() {
    let out = compile_and_run(
        r#"
fn first<T>(list: List<T>): T {
    list[0]
}
fn main() {
    print(first(["a", "b"]))
}
"#,
    );
    assert_eq!(out, vec!["a"]);
}

#[test]
fn test_codegen_generic_prepend_all() {
    let out = compile_and_run(
        r#"
fn prepend_all<T>(prefix: T, list: List<T>): List<T> {
    let mut out = []
    for x in list {
        out = out + [prefix, x]
    }
    out
}
fn main() {
    print(prepend_all(0, [1, 2, 3]))
    print(prepend_all("p", ["a", "b"]))
}
"#,
    );
    assert_eq!(out, vec!["[0, 1, 0, 2, 0, 3]", "[p, a, p, b]"]);
}

/// IR generation can leave a result slot and a jump after an arm that already
/// returned. Those instructions are unreachable; codegen must drop them
/// instead of letting Cranelift reject a block that already has a terminator.
#[test]
fn test_codegen_match_arms_that_return() {
    let output = compile_and_run(
        r#"
fn pick(v: number): number {
    match v {
        0 => { return 1 }
        _ => { return 2 }
    }
}
fn main() { print(pick(0)) }
"#,
    );
    assert_eq!(output, vec!["1"]);
}

/// String equality compares contents in the compiled path, not interned
/// pointers. An indexed string is produced at runtime, so the literal-only
/// fast path cannot hide a pointer comparison.
#[test]
fn test_codegen_string_equality_is_by_content() {
    let output = compile_and_run(
        r#"
fn main() {
    let xs = ["alpha", "beta"]
    print(xs[0] == "alpha")
    print(xs[0] == "beta")
    print("alpha" != xs[1])
}
"#,
    );
    assert_eq!(output, vec!["true", "false", "true"]);
}

/// A `+` whose operand is bound from an enum payload must stay a string
/// concatenation. The payload tag is only known at runtime, so the dynamic
/// numeric path (`xulo_as_f64`) used to rewrite the operand first and hand a
/// float to `xulo_string_concat`, which then read a bogus pointer.
#[test]
fn test_codegen_string_concat_with_match_payload_binding() {
    let output = compile_and_run(
        r#"
enum Box {
    Text(s: string)
}

fn main() {
    let b = Box::Text("hi")
    match b {
        Box::Text(s) => {
            print(s + "!")
            print("!" + s)
        }
    }
}
"#,
    );
    assert_eq!(output, vec!["hi!", "!hi"]);
}

/// The same gate for lists: a payload bound to a list keeps `+` as a list
/// concatenation instead of being folded into a number.
#[test]
fn test_codegen_list_concat_with_match_payload_binding() {
    let output = compile_and_run(
        r#"
enum Bag {
    Items(xs: List<int>)
}

fn main() {
    let b = Bag::Items([1, 2])
    match b {
        Bag::Items(xs) => print(xs + [3])
    }
}
"#,
    );
    assert_eq!(output, vec!["[1, 2, 3]"]);
}

/// A user method keeps its name when a string member has the same name.
/// `starts_with` on a struct receiver must dispatch to the `impl`, not to the
/// built-in string member.
#[test]
fn test_codegen_user_method_named_like_string_member() {
    let output = compile_and_run(
        r#"
struct Prefix { text: string }

impl Prefix {
    fn starts_with(self, other: string): bool {
        return self.text == other
    }
}

fn main() {
    let p = Prefix(text: "alpha")
    print(p.starts_with("alpha"))
    print(p.starts_with("beta"))
}
"#,
    );
    assert_eq!(output, vec!["true", "false"]);
}

/// A `let` inside a nested block shadows only that block: the outer binding is
/// visible again after it (spec `statements/let-and-assignment.md`).
#[test]
fn test_codegen_block_shadowing_restores_the_outer_binding() {
    let out = compile_and_run(
        r#"
fn main() {
    let name = "xulo"
    if name == "xulo" {
        let name = name + "!"
        print(name)
    }
    print(name)
    let mut total = 1
    for _ in 0..<2 {
        let total = total + 10
        print(total)
    }
    print(total)
}
"#,
    );
    assert_eq!(out, vec!["xulo!", "xulo", "11", "11", "1"]);
}

/// `f32`/`f64` annotations get an F64 slot, so the values print as floats
/// instead of their bit patterns as integers; `f32` stays binary32 through
/// bindings and arithmetic (T12).
#[test]
fn test_codegen_fixed_width_float_annotations_print_as_floats() {
    let out = compile_and_run(
        r#"
struct Holder { value: f32 }
fn main() {
    let single: f32 = 0.5
    let double: f64 = 2.5
    print(single * 4.0)
    print(double * 2.0)
    print(single + single)
    print(double / 2.0)
    print(single < 1.0)
    let narrow: f32 = 0.8
    print(narrow)
    let one: f32 = 1.0
    let three: f32 = 3.0
    print(one / three)
    let mut f = Holder(value: 0.8)
    print(f.value)
    f.value = 0.3
    print(f.value)
}
"#,
    );
    assert_eq!(
        out,
        vec![
            "2",
            "5",
            "1",
            "1.25",
            "true",
            "0.800000011920929",
            "0.3333333432674408",
            "0.800000011920929",
            "0.30000001192092896",
        ]
    );
}

/// `Error("text")` builds the built-in error object, so `?` propagation and
/// `e.message` work on the compiled path (spec error-handling.md).
#[test]
fn test_codegen_error_constructor_and_propagation() {
    let out = compile_and_run(
        r#"
fn parse(v: int): Result<int, Error> {
    if v < 0 {
        return Result::Err(Error("bad input"))
    }
    Result::Ok(v)
}

fn double(v: int): Result<int, Error> {
    let x = parse(v)?
    Result::Ok(x * 2)
}

fn main() {
    match double(21) {
        Result::Ok(v) => print(v)
        Result::Err(e) => print(e.message)
    }
    match double(-1) {
        Result::Ok(_) => print("ok")
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out, vec!["42", "bad input"]);
}
