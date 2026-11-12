//! LLVM vs Cranelift differential tests.
//!
//! `llvm` is the compile-time feature; `XULO_LLVM=1` is the runtime switch
//! (section 9 of docs/llvm-backend.md). When the feature is disabled the file
//! is empty; when the switch is unset the tests are skipped to keep the normal
//! suite fast.
#![cfg(feature = "llvm")]

use std::path::{Path, PathBuf};

use xulo_compiler::aot::{AotBackend, BuildProfile, build_executable_with_profile};
use xulo_compiler::{codegen::CodeGen, compile_to_ir};

fn enabled() -> bool {
    std::env::var_os("XULO_LLVM").is_some()
}

const CORPUS: &[(&str, &str)] = &[
    (
        "arithmetic",
        r#"
        fn main() {
            print(1 + 2 * 3)
            print(10 - 4)
            print(7 / 2)
            print(7 % 4)
            print(2 ** 10)
            print(-5)
        }
        "#,
    ),
    (
        "floats",
        r#"
        fn main() {
            print(1.5 + 2.25)
            print(10.0 / 4.0)
            print(10.5 % 3.0)
            print(2.0 ** 3.0)
        }
        "#,
    ),
    (
        "comparisons_and_logic",
        r#"
        fn main() {
            print(1 < 2)
            print(1.5 >= 2.5)
            print(true and false)
            print(true or false)
            print(!true)
            print(1 == 1.0)
        }
        "#,
    ),
    (
        "bit_ops_and_shifts",
        r#"
        fn main() {
            print(6 & 3)
            print(6 | 3)
            print(6 ^ 3)
            print(1 << 4)
            print(16 >> 2)
            print(~5)
        }
        "#,
    ),
    (
        "control_flow",
        r#"
        fn classify(n: number): string {
            if n < 0 { return "neg" }
            if n == 0 { return "zero" }
            "pos"
        }
        fn main() {
            print(classify(-1))
            print(classify(0))
            print(classify(3))
            let mut i = 0
            while i < 3 {
                print(i)
                i = i + 1
            }
        }
        "#,
    ),
    (
        "functions_and_recursion",
        r#"
        fn fib(n: number): number {
            if n < 2 { return n }
            fib(n - 1) + fib(n - 2)
        }
        fn main() { print(fib(10)) }
        "#,
    ),
    (
        "arrays_and_objects",
        r#"
        fn main() {
            let mut arr = [1, 2, 3]
            arr[1] = 9
            print(arr[0])
            print(arr[1])
            print(arr.len)
            let o = { name: "x", n: 2 }
            print(o.name)
            print(o.n)
        }
        "#,
    ),
    (
        "strings",
        r#"
        fn main() {
            let s = "Hello" + ", " + "world"
            print(s)
            print(s.len)
            print("hello".slice(1, 4))
            print("abc".code_at(1))
        }
        "#,
    ),
    (
        "string_builder",
        r#"
        fn main() {
            let mut b = StringBuilder::create()
            b.append("hello")
            b.append(", ")
            b.append("builder")
            let s = b.build()
            print(s)
            print(s.len)
        }
        "#,
    ),
    (
        "closures",
        r#"
        fn main() {
            let base = 100
            let add = fn(x: number): number { x + base }
            print(add(5))
            let twice = (x: number): number => x * 2
            print(twice(21))
        }
        "#,
    ),
    (
        "match_and_enum",
        r#"
        enum Shape {
            Circle(radius: number),
            Square(side: number),
        }
        fn area(s: Shape): number {
            match s {
                Shape::Circle(r) => 3 * r * r,
                Shape::Square(x) => x * x,
            }
        }
        fn main() {
            print(area(Shape::Circle(2)))
            print(area(Shape::Square(3)))
        }
        "#,
    ),
    (
        "ffi_abs",
        r#"
        extern "C" {
            fn abs(x: i32): i32
        }
        fn main() { print(abs(-42)) }
        "#,
    ),
];

fn jit_output(source: &str) -> Vec<String> {
    let ir = compile_to_ir(source, Path::new("diff.xulo")).expect("IR generation failed");
    let mut codegen = CodeGen::new();
    let ptr = codegen.compile(&ir).expect("JIT codegen failed");
    unsafe {
        let func: fn() = std::mem::transmute(ptr);
        func();
    }
    xulo_runtime::runtime::xulo_take_output()
}

fn unique_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("xulo-llvm-diff-{}-{nanos}", name));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn aot_output(name: &str, source: &str, backend: AotBackend) -> std::process::Output {
    let dir = unique_dir(name);
    let src = dir.join("main.xulo");
    let exe = dir.join("main");
    std::fs::write(&src, source).unwrap();
    let built = build_executable_with_profile(&src, &exe, backend, BuildProfile::Debug)
        .unwrap_or_else(|e| panic!("{} build failed for `{name}`: {e}", backend.name()));
    let run = std::process::Command::new(&built.executable)
        .output()
        .expect("failed to run executable");
    let _ = std::fs::remove_dir_all(&dir);
    run
}

fn llvm_output(name: &str, source: &str) -> Vec<String> {
    let run = aot_output(name, source, AotBackend::Llvm);
    assert!(
        run.status.success(),
        "LLVM executable failed for `{name}`: {run:?}"
    );
    String::from_utf8_lossy(&run.stdout)
        .lines()
        .map(|line| line.to_string())
        .collect()
}

#[test]
fn llvm_matches_cranelift_on_corpus() {
    if !enabled() {
        eprintln!("skipping LLVM differential tests (set XULO_LLVM=1 to run)");
        return;
    }

    for (name, source) in CORPUS {
        let jit = jit_output(source);
        let llvm = llvm_output(name, source);
        assert_eq!(llvm, jit, "backend divergence for `{name}`");
    }
}

#[test]
fn llvm_matches_cranelift_on_trap_exit_code() {
    if !enabled() {
        return;
    }
    for (name, source) in [
        ("div", "fn main() {\n print(1 / 0)\n}\n"),
        ("pow_overflow", "fn main() {\n print(2 ** 10000)\n}\n"),
    ] {
        let cranelift = aot_output(&format!("{name}-cranelift"), source, AotBackend::Cranelift);
        let llvm = aot_output(&format!("{name}-llvm"), source, AotBackend::Llvm);
        assert_eq!(
            llvm.status.code(),
            cranelift.status.code(),
            "`{name}` trap exit code diverged: llvm={llvm:?} cranelift={cranelift:?}"
        );
        assert_eq!(
            llvm.stdout, cranelift.stdout,
            "`{name}` stdout before trap diverged"
        );
    }
}
