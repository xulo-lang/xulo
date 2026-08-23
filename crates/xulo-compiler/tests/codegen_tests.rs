use std::path::Path;
use xulo_compiler::{compile_to_ir, codegen::CodeGen};

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
    compile_and_run(r#"
fn main() {
    let x = (1 + 2) * 3
    let y = 10 - (2 + 3)
}
"#);
}

#[test]
fn test_codegen_array_literal() {
    compile_and_run(r#"
fn main() {
    let arr = [1, 2, 3]
}
"#);
}

#[test]
fn test_codegen_object_literal() {
    compile_and_run(r#"
fn main() {
    let obj = { name: "test", value: 42 }
}
"#);
}

#[test]
fn test_codegen_closure() {
    compile_and_run(r#"
fn main() {
    let add = fn(a, b) { return a + b }
}
"#);
}

#[test]
fn test_codegen_list_push() {
    compile_and_run(r#"
fn main() {
    let arr = [1, 2, 3]
    arr.push(4)
}
"#);
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
    let out = compile_and_run(r#"
fn main() {
    let x = 100
    print(x)
}
"#);
    assert_eq!(out, vec!["100"]);
}

#[test]
fn test_codegen_multiple_prints() {
    let out = compile_and_run(r#"
fn main() {
    print("first")
    print("second")
}
"#);
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
    assert_eq!(out, vec!["{a: 1, b: 2}"]);
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
    let out = compile_and_run(r#"
fn main() {
    let a = [1, 2]
    let b = [3, 4]
    print(a + b)
}
"#);
    assert_eq!(out, vec!["[1, 2, 3, 4]"]);
}

// -- for loops --

#[test]
fn test_codegen_for_loop_integers() {
    let out = compile_and_run(r#"
fn main() {
    for i in [10, 20, 30] {
        print(i)
    }
}
"#);
    assert_eq!(out, vec!["10", "20", "30"]);
}

#[test]
fn test_codegen_for_loop_strings() {
    let out = compile_and_run(r#"
fn main() {
    for x in ["a", "b", "c"] {
        print(x)
    }
}
"#);
    assert_eq!(out, vec!["a", "b", "c"]);
}

#[test]
fn test_codegen_for_loop_concat() {
    let out = compile_and_run(r#"
fn main() {
    let mut out = []
    for x in [1, 2, 3] {
        out = out + [x]
    }
    print(out)
}
"#);
    assert_eq!(out, vec!["[1, 2, 3]"]);
}

#[test]
fn test_codegen_for_loop_concat_strings() {
    let out = compile_and_run(r#"
fn main() {
    let mut out = []
    for x in ["a", "b"] {
        out = out + [x]
    }
    print(out)
}
"#);
    assert_eq!(out, vec!["[a, b]"]);
}

// -- generic functions --

#[test]
fn test_codegen_generic_identity_int() {
    let out = compile_and_run(r#"
fn identity<T>(value: T): T {
    value
}
fn main() {
    print(identity(42))
}
"#);
    assert_eq!(out, vec!["42"]);
}

#[test]
fn test_codegen_generic_identity_string() {
    let out = compile_and_run(r#"
fn identity<T>(value: T): T {
    value
}
fn main() {
    print(identity("hello"))
}
"#);
    assert_eq!(out, vec!["hello"]);
}

#[test]
fn test_codegen_generic_identity_bool() {
    let out = compile_and_run(r#"
fn identity<T>(value: T): T {
    value
}
fn main() {
    print(identity(true))
}
"#);
    // 注意：泛型函数返回 bool 时，运行时无法区分 bool 和 int（true=1, false=0）
    // 因此输出为 "1" 而非 "true"
    assert_eq!(out, vec!["1"]);
}

#[test]
fn test_codegen_generic_twice_int() {
    let out = compile_and_run(r#"
fn twice<T>(value: T): list<T> {
    [value, value]
}
fn main() {
    print(twice(7))
}
"#);
    assert_eq!(out, vec!["[7, 7]"]);
}

#[test]
fn test_codegen_generic_twice_string() {
    let out = compile_and_run(r#"
fn twice<T>(value: T): list<T> {
    [value, value]
}
fn main() {
    print(twice("x"))
}
"#);
    assert_eq!(out, vec!["[x, x]"]);
}

#[test]
fn test_codegen_generic_first() {
    let out = compile_and_run(r#"
fn first<T>(list: list<T>): T {
    list[0]
}
fn main() {
    print(first([10, 20, 30]))
}
"#);
    assert_eq!(out, vec!["10"]);
}

#[test]
fn test_codegen_generic_first_string() {
    let out = compile_and_run(r#"
fn first<T>(list: list<T>): T {
    list[0]
}
fn main() {
    print(first(["a", "b"]))
}
"#);
    assert_eq!(out, vec!["a"]);
}

#[test]
fn test_codegen_generic_prepend_all() {
    let out = compile_and_run(r#"
fn prependAll<T>(prefix: T, list: list<T>): list<T> {
    let mut out = []
    for x in list {
        out = out + [prefix, x]
    }
    out
}
fn main() {
    print(prependAll(0, [1, 2, 3]))
    print(prependAll("p", ["a", "b"]))
}
"#);
    assert_eq!(out, vec!["[0, 1, 0, 2, 0, 3]", "[p, a, p, b]"]);
}
