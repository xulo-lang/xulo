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

// -- basic closure capture --

#[test]
fn closure_captures_outer_let() {
    let out = compile_and_run(
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
fn arrow_closure_captures_outer_let() {
    let out = compile_and_run(
        r#"
fn main() {
    let n = 7
    let double = (x: number): number => x * 2 + n
    print(double(1))
}
"#,
    );
    assert_eq!(out, vec!["9"]);
}

#[test]
fn closure_captures_multiple_vars() {
    let out = compile_and_run(
        r#"
fn main() {
    let a = 10
    let b = 20
    let sum = fn(): number { a + b }
    print(sum())
}
"#,
    );
    assert_eq!(out, vec!["30"]);
}

#[test]
fn closure_without_capture() {
    let out = compile_and_run(
        r#"
fn main() {
    let square = fn(x: number): number { x * x }
    print(square(6))
}
"#,
    );
    assert_eq!(out, vec!["36"]);
}

// -- shadowing / mutation --

#[test]
fn closure_reads_captured_value_at_call_time() {
    let out = compile_and_run(
        r#"
fn main() {
    let mut x = 1
    let get = fn(): number { x }
    x = 2
    print(get())
}
"#,
    );
    assert_eq!(out, vec!["2"]);
}

// -- higher-order functions --

#[test]
fn closure_passed_as_argument() {
    let out = compile_and_run(
        r#"
fn apply(f: fn(number): number, v: number): number {
    f(v)
}

fn main() {
    let inc = fn(x: number): number { x + 1 }
    print(apply(inc, 41))
}
"#,
    );
    assert_eq!(out, vec!["42"]);
}

#[test]
fn closure_returned_from_function() {
    let out = compile_and_run(
        r#"
fn make_adder(n: number): fn(number): number {
    fn(x: number): number { x + n }
}

fn main() {
    let add5 = make_adder(5)
    print(add5(37))
}
"#,
    );
    assert_eq!(out, vec!["42"]);
}

// -- recursion / self reference --

#[test]
fn inner_fn_recurses() {
    let out = compile_and_run(
        r#"
fn main() {
    fn fact(n: number): number {
        if n <= 1 {
            1
        } else {
            n * fact(n - 1)
        }
    }
    print(fact(5))
}
"#,
    );
    assert_eq!(out, vec!["120"]);
}

// -- loop capture --

#[test]
fn closures_capture_loop_bindings() {
    let out = compile_and_run(
        r#"
fn main() {
    let mut fns: List<fn(): number> = []
    for i in [1, 2, 3] {
        let f = fn(): number { i }
        fns.push(f)
    }
    for f in fns {
        print(f())
    }
}
"#,
    );
    assert_eq!(out, vec!["1", "2", "3"]);
}

// -- nested closures --

#[test]
fn nested_closure_captures_both_scopes() {
    let out = compile_and_run(
        r#"
fn main() {
    let outer = 10
    let make = fn(m: number): fn(): number {
        fn(): number { m + outer }
    }
    let f = make(5)
    print(f())
}
"#,
    );
    assert_eq!(out, vec!["15"]);
}

// -- closure as printable value --

#[test]
fn printing_a_closure_shows_function_tag() {
    let out = compile_and_run(
        r#"
fn main() {
    let f = fn(): number { 1 }
    print(f)
}
"#,
    );
    assert_eq!(out, vec!["<function>"]);
}

// -- top-level function as value --

#[test]
fn top_level_function_used_as_value() {
    let out = compile_and_run(
        r#"
fn double(x: number): number {
    x * 2
}

fn main() {
    let f = double
    print(f(21))
}
"#,
    );
    assert_eq!(out, vec!["42"]);
}

// -- float capture --

#[test]
fn closure_captures_float() {
    let out = compile_and_run(
        r#"
fn main() {
    let scale = 1.5
    let f = fn(x: number): number { x * scale }
    print(f(2))
}
"#,
    );
    assert_eq!(out, vec!["3"]);
}
