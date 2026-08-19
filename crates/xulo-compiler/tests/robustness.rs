//! Robustness / fuzz tests: the compiler must never panic — malformed input
//! always surfaces as a `Result::Err` (a located diagnostic), never a crash.

use std::panic::{self, AssertUnwindSafe};
use std::path::Path;

fn compile_quiet(src: &str) {
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        let _ = xulo_compiler::compile(src, Path::new("fuzz.xulo"));
    }));
    assert!(
        outcome.is_ok(),
        "compile panicked on input:\n```\n{src}\n```"
    );
}

/// A corner-case corpus that historically exercised risky parser paths:
/// unbalanced delimiters, unterminated string/comment literals, unicode,
/// deep nesting, null bytes, and token soup.
#[test]
fn adversarial_corpus_does_not_panic() {
    let corpus = [
        "",
        " ",
        "\u{0000}",
        "\u{1f600}",
        "========================",
        "fn",
        "fn main( {",
        "fn main() {",
        "fn main() { ;; ;; }",
        "fn main() { print(\"unterminated }",
        "/* unterminated",
        "fn main() { let x = /* unterminated",
        "fn main() { print('single') }",
        "fn main() { print('unterminated) }",
        "[][][][]]]",
        "((((((((((((((((((((((((((((((((((((((((((((((((((((((",
        "))))))))))))))))))))))))))))))))))))))))))))))))))))))",
        "fn main() { { { { { { } } } } } }",
        "fn main() { let s = \"\\\" \\n \\t \\u{110000}\" }",
        "1.2.3.4",
        "0x1f 0b101 1e999",
        "fn main() { let x = 1 2 3 }",
        "fn main() { let x = x.x.x.x.x.x.x.x }",
        "enum E { A( fn main() { B }",
        "match 1 { _ => 1 }",
        "fn main() { match 1 { _ => } }",
        "@State let x = 1",
        "View { }",
        "$ $ $",
        "async fn f() { await await await x }",
        "fn main() { try { } catch (e) { } }",
        "fn main() { for i in 0..<5 }",
        "fn main() { while true }",
        "type T = T & T | T",
        "fn main() { let f: fn(a: number): number = fn(x: number): number { x } print(f(1)) }",
        "import { A as } from \"./x\"",
        "export default",
        "\u{3000}\u{3000}fn\u{3000}main\u{3000}()\u{3000}{\u{3000}}",
        "fn \u{1f600}() {}",
        "foo.bar.baz(1)(2)(3)",
        "fn main() { print(1 == == 2) }",
        "fn main() { let a = { b: {} -> } }",
        "fn main() { print(\"\\u{\") }",
        "fn main() { let x = 1 ??? 2 }",
        "fn main() { match 1 { 1 => 2, } }",
        "fn main() { if true { while true { for i in 0..<5 { match 1 { _ => 1 } } } } }",
        "@Store const { } = f()",
        "enum E { A( ) }",
        "fn main() { let f: fn( = x }",
        "a?.?.b",
        "0..<..<1",
        "/* outer /* inner */ still",
        "fn main() { let a = [1, 2, _] }",
        "? : ? : ?",
        "fn main() { print(''' ') }",
        "$ $ $ $ $ $ $ $",
        "enum E<T,U> { A(T, U) B(list<T>) }",
    ];
    for src in corpus {
        compile_quiet(src);
    }
}

/// Deterministic token-soup fuzzing: pseudo-random strings drawn from the
/// lexer's own vocabulary, fed through the whole pipeline.
#[test]
fn token_soup_fuzz_does_not_panic() {
    fn next(seed: &mut u64) -> u64 {
        // xorshift64
        let mut x = *seed;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *seed = x;
        x
    }

    let atoms = [
        "fn",
        "let",
        "const",
        "print",
        "if",
        "else",
        "while",
        "for",
        "in",
        "return",
        "match",
        "enum",
        "type",
        "import",
        "export",
        "from",
        "async",
        "await",
        "try",
        "catch",
        "throw",
        "null",
        "true",
        "false",
        "View",
        "Screen",
        "@State",
        "@Store",
        "@Effect",
        "@Environment",
        "$name",
        "xulomain",
        "_",
        "$",
        "str",
        "{",
        "}",
        "(",
        ")",
        "[",
        "]",
        ":",
        ";",
        ",",
        ".",
        "\"s\"",
        "42",
        "3.14",
        "0..<5",
        "::",
        "?.",
        "??",
        "=>",
        "->",
        "==",
        "!=",
        "<",
        ">",
        "<=",
        ">=",
        "&&",
        "+",
        "-",
        "*",
        "/",
        "?:",
        "=",
        "foo",
        "bar",
        "baz",
        "T",
        "U",
        "string",
        "number",
        "boolean",
        "list",
        "object",
        "\\n",
        "\\t",
        " ",
        "  ",
        "世",
        "😀",
    ];

    for (tries, mut seed) in [(3000, 0xC0FFEE_u64), (2000, 0xFEEDFACE_u64)] {
        for _ in 0..tries {
            let len = (next(&mut seed) % 24) as usize;
            let mut src = String::new();
            for _ in 0..len {
                let idx = (next(&mut seed) as usize) % atoms.len();
                src.push_str(atoms[idx]);
                src.push(' ');
            }
            compile_quiet(&src);
        }
    }
}

/// Stress deep nesting of balanced delimiters through every stage.
///
/// The test harness gives each `#[test]` thread a 2 MiB stack, which debug
/// builds of the recursive parsers can exhaust at moderate depth, so the
/// compiles run on an explicit large-stack thread.
#[test]
fn deep_nesting_does_not_panic() {
    fn on_big_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap();
    }

    let depth = 120;
    let open = "(".repeat(depth);
    let close = ")".repeat(depth);
    let parens = format!("fn main() {{ print({open}1{close}) }}");
    on_big_stack(move || compile_quiet(&parens));

    let braces = format!("fn main() {{{}print(1){}}}", "{".repeat(60), "}".repeat(60));
    on_big_stack(move || compile_quiet(&braces));

    let ifs = format!(
        "fn main() {{ {} print(1) {} }}",
        "if true { ".repeat(40),
        " }".repeat(40)
    );
    on_big_stack(move || compile_quiet(&ifs));

    let ui = format!(
        "fn main(): View {{ {} Text(\"x\") {} }}",
        "Screen { ".repeat(40),
        " }".repeat(40)
    );
    on_big_stack(move || compile_quiet(&ui));

    let ui_else_if = format!(
        "fn main(): View {{ Screen {{ {} Text(\"x\") {} }} }}",
        "if true { ".repeat(40),
        " } else { ".repeat(39) + " }"
    );
    on_big_stack(move || compile_quiet(&ui_else_if));
}

/// Beyond the parser's nesting budget the compiler must reject with a clean
/// diagnostic, never crash (run on a large stack so only genuinely unbounded
/// recursion could overflow it).
#[test]
fn extreme_nesting_returns_error_not_crash() {
    let src = format!(
        "fn main() {{ print({}1{}) }}",
        "(".repeat(10_000),
        ")".repeat(10_000)
    );
    let outcome = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            panic::catch_unwind(AssertUnwindSafe(move || {
                xulo_compiler::compile(&src, Path::new("fuzz.xulo"))
            }))
        })
        .unwrap()
        .join()
        .unwrap()
        .expect("extreme nesting must not panic");
    let err = outcome.unwrap_err();
    assert!(
        err.message.contains("nesting is too deep"),
        "expected nesting error, got: {}",
        err.message
    );
}

/// UI component nesting must hit the same depth budget as statements and
/// expressions: hostile `VStack { VStack { ... } }` input (which recurses
/// through `ui_elements`, historically unguarded) must be rejected with a
/// diagnostic, never overflow the stack.
#[test]
fn extreme_ui_nesting_returns_error_not_crash() {
    let src = format!(
        "fn main(): View {{ {} Text(\"x\") {} }}",
        "Screen { ".repeat(10_000),
        " }".repeat(10_000)
    );
    let outcome = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            panic::catch_unwind(AssertUnwindSafe(move || {
                xulo_compiler::compile(&src, Path::new("fuzz.xulo"))
            }))
        })
        .unwrap()
        .join()
        .unwrap()
        .expect("extreme UI nesting must not panic");
    let err = outcome.unwrap_err();
    assert!(
        err.message.contains("nesting is too deep"),
        "expected nesting error, got: {}",
        err.message
    );
}

/// A dependent module dispatching `Trait::method` on an impl declared in an
/// imported module must produce a bundle where impls register into (and
/// dispatch through) a single shared `__impls` registry declared once at the
/// top, before any module IIFE runs.
#[test]
fn cross_module_trait_dispatch_bundle_emits_shared_registry() {
    let dir = std::env::temp_dir().join(format!(
        "xulo_cg_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("shapes.xulo"),
        r#"
        pub trait Shape { fn area(self): number }
        pub type Rect = object
        impl Shape for Rect {
            fn area(self): number { return self.w * self.h }
        }
        pub fn rect(w: number, h: number): Rect {
            let r = { w: w, h: h }
            r
        }
        "#,
    )
    .unwrap();
    std::fs::write(
        dir.join("main.xulo"),
        r#"
        import type { Shape } from "./shapes"
        import type { Rect } from "./shapes"
        import { rect } from "./shapes"
        fn main() { print(Shape::area(rect(3, 4))) }
        "#,
    )
    .unwrap();

    let (js, _) = xulo_compiler::module::compile_file(&dir.join("main.xulo")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        js.starts_with("const __impls = {};\n"),
        "registry must precede every module IIFE:\n{js}"
    );
    assert!(
        js.contains("__impls[\"impl_Shape_Rect_area\"] = function (self) {"),
        "impl must register into the shared registry:\n{js}"
    );
    // One declaration, two references (registration + dispatch).
    assert_eq!(js.matches("const __impls").count(), 1, "js:\n{js}");
    assert_eq!(
        js.matches("__impls[\"impl_Shape_Rect_area\"]").count(),
        2,
        "js:\n{js}"
    );
}
