//! FFI end-to-end through the native interpreter (lex → parse → semantic →
//! run): extern imports register at declaration time (dlsym + generated C
//! trampoline) and call exactly like a declared `fn` at the call site —
//! named arguments reorder, an omitted `Ptr?` binds `null` (spec ffi.md
//! "Calling"). Foreign symbols come from the process itself (libc/libm).

use xulo_core::error::XuloError;
use xulo_parser::parse_program;
use xulo_runtime::interpreter::Interpreter;

/// Full pipeline: lex -> parse -> semantic check -> run.
fn run(src: &str) -> Result<Vec<String>, XuloError> {
    let program = parse_program(src).expect("parse failed");
    xulo_semantic::analyze(&program)?;
    let interp = Interpreter::new();
    interp.run(&program)
}

fn run_ok(src: &str) -> Vec<String> {
    run(src).unwrap_or_else(|e| panic!("{}", e.message))
}

fn err_of(src: &str) -> XuloError {
    run(src).expect_err("expected an error")
}

#[test]
fn extern_alias_calls_declared_symbol() {
    // `fn abs … as c_abs`: the declared name is the C symbol, the alias the
    // local binding (spec ffi.md).
    let out = run_ok(
        r#"
        extern "C" { fn abs(x: i32): i32 as c_abs }
        fn main() { print(c_abs(-42)) }
        "#,
    );
    assert_eq!(out, ["42"]);
}

#[test]
fn extern_f64_roundtrip() {
    let out = run_ok(
        r#"
        extern "C" { fn sqrt(x: f64): f64 }
        fn main() { print(sqrt(9)) }
        "#,
    );
    assert_eq!(out, ["3"]);
}

#[test]
fn extern_i64_roundtrip() {
    let out = run_ok(
        r#"
        extern "C" { fn labs(x: i64): i64 as c_labs }
        fn main() { print(c_labs(-9000000000)) }
        "#,
    );
    assert_eq!(out, ["9000000000"]);
}

#[test]
fn extern_narrow_u8_return_reads_width_only() {
    // `isalnum` returns a C int (a ctype bitmask — nonzero for `'A'`);
    // declared `u8` the trampoline reads one byte and the caller keeps it
    // unsigned, so `'A'` stays `8` and `'#'` stays `0`.
    let out = run_ok(
        r#"
        extern "C" { fn isalnum(c: i32): u8 as c_isalnum }
        fn main() {
            print(c_isalnum(65) != 0)
            print(c_isalnum(33) != 0)
        }
        "#,
    );
    assert_eq!(out, ["true", "false"]);
}

#[test]
fn extern_bool_return() {
    let out = run_ok(
        r#"
        extern "C" { fn isalnum(c: i32): bool as c_isalnum_b }
        fn main() { print(c_isalnum_b(65)) }
        "#,
    );
    assert_eq!(out, ["true"]);
}

#[test]
fn extern_f32_roundtrip() {
    let out = run_ok(
        r#"
        extern "C" { fn cosf(x: f32): f32 as c_cosf }
        fn main() { print(c_cosf(0)) }
        "#,
    );
    assert_eq!(out, ["1"]);
}

#[test]
fn extern_void_return_is_null() {
    let out = run_ok(
        r#"
        extern "C" { fn srand(seed: u32) }
        fn main() {
            srand(1)
            print("sowed")
        }
        "#,
    );
    assert_eq!(out, ["sowed"]);
}

#[test]
fn extern_ptr_roundtrip_malloc_free() {
    let out = run_ok(
        r#"
        extern "C" {
            fn malloc(size: usize): Ptr
            fn free(p: Ptr)
        }
        fn main() {
            let p = malloc(64)
            free(p)
            print("freed")
        }
        "#,
    );
    assert_eq!(out, ["freed"]);
}

#[test]
fn extern_ptr_display_and_identity() {
    let out = run_ok(
        r#"
        extern "C" {
            fn malloc(size: usize): Ptr
            fn free(p: Ptr)
        }
        fn main() {
            let a = malloc(8)
            let b = a
            print(a == b)
            print(a)
            free(a)
        }
        "#,
    );
    assert_eq!(out.len(), 2);
    assert_eq!(out[0], "true");
    assert!(out[1].starts_with("<ptr 0x"), "got {}", out[1]);
    assert!(out[1].ends_with('>'), "got {}", out[1]);
}

#[test]
fn extern_nullable_ptr_narrowing_and_omitted_param_binds_null() {
    // `p != null` narrows; the omitted `Ptr?` argument to `free` binds
    // `null` (spec ffi.md Calling) — `free(NULL)` is a libc no-op.
    let out = run_ok(
        r#"
        extern "C" {
            fn malloc(size: usize): Ptr?
            fn free(p: Ptr?)
        }
        fn main() {
            let p = malloc(32)
            if p != null {
                free()
                print("ok")
            }
        }
        "#,
    );
    assert_eq!(out, ["ok"]);
}

#[test]
fn extern_named_arguments_reorder_by_declaration() {
    // Written out of order; the values land by declared parameter name.
    let out = run_ok(
        r#"
        extern "C" { fn pow(x: f64, y: f64): f64 }
        fn main() { print(pow(y: 10, x: 2)) }
        "#,
    );
    assert_eq!(out, ["1024"]);
}

#[test]
fn extern_missing_symbol_fails_at_registration() {
    // Registration runs before any statement — the error precedes side effects.
    let err = err_of(
        r#"
        extern "C" { fn missingXuloFn(): i32 }
        fn main() { print("never") }
        "#,
    );
    assert!(
        err.message.contains("not found in the process"),
        "got {}",
        err.message
    );
}

#[test]
fn extern_arity_beyond_trampoline_is_rejected_at_registration() {
    let err = err_of(
        r#"
        extern "C" {
            fn sevenParams(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32)
        }
        fn main() { print("never") }
        "#,
    );
    assert!(err.message.contains("at most 6"), "got {}", err.message);
}

#[test]
fn extern_import_cannot_be_used_as_a_value() {
    // E0903: direct call position only (spec ffi.md "Calling").
    let err = err_of(
        r#"
        extern "C" { fn abs(x: i32): i32 as c_abs }
        fn main() { let g = c_abs }
        "#,
    );
    assert!(
        err.message.contains("only be called directly"),
        "got {}",
        err.message
    );
}

#[test]
fn pub_extern_block_exports_its_bindings() {
    let interp = Interpreter::new();
    let program = parse_program(r#"pub extern "C" { fn abs(x: i32): i32 as cAbs }"#).unwrap();
    let exports = interp
        .exec_module(&program, &[], false)
        .unwrap_or_else(|e| match e {
            xulo_runtime::interpreter::RunError::Err(e) => panic!("{}", e.message),
            xulo_runtime::interpreter::RunError::Propagate(_) => panic!("stray propagate"),
            xulo_runtime::interpreter::RunError::Throw(_) => panic!("uncaught throw"),
        });
    assert!(
        exports.bindings.iter().any(|(n, _)| n == "cAbs"),
        "cAbs not exported: {:?}",
        exports.bindings.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
}

#[test]
fn extern_string_is_not_a_ptr() {
    // There is no string → Ptr coercion (spec ffi.md): passing a string to a
    // `Ptr` parameter is a compile-time type error, not a runtime bridge.
    let err = err_of(
        r#"
        extern "C" { fn strlen(s: Ptr): usize }
        fn main() { print(strlen("hello")) }
        "#,
    );
    assert!(err.message.contains("Ptr"), "got {}", err.message);
}
