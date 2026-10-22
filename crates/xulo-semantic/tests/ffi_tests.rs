//! FFI semantic checks (spec `ffi.md`): boundary types (`E0901`), the ABI
//! registry (`E0902`), the direct-call-only restriction on imports
//! (`E0903`), registration/visibility of import blocks and exports, and the
//! fixed-bit literal/arithmetic rules (spec `types/primitive-types.md`)
//! an extern signature depends on.

use xulo_core::error::XuloError;
use xulo_parser::parse_program;

fn analyze_src(src: &str) -> Result<(), XuloError> {
    let program = parse_program(src)?;
    xulo_semantic::analyze(&program)
}

fn err_of(src: &str) -> XuloError {
    match analyze_src(src) {
        Ok(()) => panic!("expected error for {src:?}"),
        Err(e) => e,
    }
}

fn ok(src: &str) {
    if let Err(e) = analyze_src(src) {
        panic!("expected OK for {src:?}: {} ({:?})", e.message, e.code());
    }
}

// ---------------------------------------------------------------
// Accepted forms (the spec's own examples)
// ---------------------------------------------------------------

#[test]
fn spec_import_block_and_export_example_analyze() {
    ok(r#"
        extern "C" {
          fn abs(x: i32): i32
          fn getpid(): i32
          fn malloc(size: usize): Ptr? as cMalloc
          fn free(p: Ptr?) as cFree
        }

        pub extern "C" fn sum(xs: i32, n: i32): i32 {
          let mut total: i32 = 0
          let mut i: i32 = 0
          while i < n {
            total = total + abs(xs + i)
            i = i + 1
          }
          total
        }

        fn main() { print(sum(5, 3)) }
    "#);
}

#[test]
fn ptr_nullable_narrowing_forms_analyze() {
    // spec ffi.md "Handling null": branch-local narrowing, `??`, and an
    // ordinary `Ptr` value flowing through Xulo code.
    ok(r#"
        extern "C" {
          fn malloc(size: usize): Ptr? as cMalloc
          fn free(p: Ptr?) as cFree
        }

        fn store(size: usize) {
          let p = cMalloc(size)
          if p != null {
            cFree(p)
          } else {
            print("out of memory")
          }
        }

        fn mustAllocate(size: usize): Ptr {
          let p = cMalloc(size) ?? panic("out of memory")
          p
        }

        fn roundTrip(p: Ptr): Ptr? { p }
    "#);
}

#[test]
fn all_abi_safe_types_are_accepted() {
    ok(r#"
        extern "C" {
          fn takesAll(b: bool, a: i8, c: i16, d: i32, e: i64,
                      f: u8, g: u16, h: u32, i: u64,
                      j: f32, k: f64, l: usize, m: isize, n: Ptr, o: Ptr?)
        }
        extern "C" { fn nothing(): unit }
        pub extern "C" fn ext(b: bool, p: Ptr?): u64 { 1 }
    "#);
}

#[test]
fn alias_naming_a_table_type_is_accepted() {
    ok(r#"
        type Byte = u8
        extern "C" { fn f(b: Byte): Byte }
    "#);
}

#[test]
fn system_abi_is_registered() {
    ok(r#"extern "system" { fn getpid(): i32 }"#);
}

#[test]
fn direct_calls_and_ordinary_uses_analyze() {
    ok(r#"
        extern "C" { fn abs(x: i32): i32 }
        fn main() {
          let n = abs(-1)
          print(n)
        }
    "#);
}

#[test]
fn pub_export_is_ordinary_for_value_uses() {
    // spec ffi.md: the export is an ordinary module-level `fn`; the
    // direct-call restriction applies only to imports.
    ok(r#"
        pub extern "C" fn add(a: i32, b: i32): i32 { a + b }
        fn main() {
          let f = add
          print(f(1, 2))
        }
    "#);
}

#[test]
fn shadowing_local_binding_is_not_e0903() {
    ok(r#"
        extern "C" { fn abs(x: i32): i32 }
        fn main() {
          let abs = 5
          let g = abs
          print(g)
        }
    "#);
}

// ---------------------------------------------------------------
// E0901 — types at the boundary
// ---------------------------------------------------------------

#[test]
fn e0901_string_parameter() {
    let err = err_of(r#"extern "C" { fn bad(s: string): i32 }"#);
    assert_eq!(err.code(), "E0901");
    assert!(
        err.message
            .contains("`string` is not allowed across an `extern \"C\"` boundary"),
        "message: {}",
        err.message
    );
    assert!(
        err.note
            .as_deref()
            .is_some_and(|n| n.contains("ABI-safe types:")),
        "expected the ABI-safe-types note, got {:?}",
        err.note
    );
}

#[test]
fn e0901_optional_non_ptr_types() {
    assert_eq!(
        err_of(r#"extern "C" { fn f(n: i32?): i32 }"#).code(),
        "E0901"
    );
    assert_eq!(
        err_of(r#"extern "C" { fn f(n: string?): i32 }"#).code(),
        "E0901"
    );
}

#[test]
fn e0901_unit_parameter() {
    let err = err_of(r#"extern "C" { fn bad(n: unit): unit }"#);
    assert_eq!(err.code(), "E0901");
    // `unit` stays allowed as the return type.
    ok(r#"extern "C" { fn good(n: i32): unit }"#);
}

#[test]
fn e0901_user_facing_numeric_layer_is_excluded() {
    for ty in ["number", "int", "float", "null", "unknown"] {
        let src = format!("extern \"C\" {{ fn f(x: {ty}): i32 }}");
        assert_eq!(
            err_of(&src).code(),
            "E0901",
            "{ty} should not cross the boundary"
        );
    }
}

#[test]
fn e0901_collections_structs_and_unions() {
    assert_eq!(
        err_of(r#"extern "C" { fn f(x: List<i32>): i32 }"#).code(),
        "E0901"
    );
    assert_eq!(
        err_of(r#"extern "C" { fn f(x: nosuchtype): i32 }"#).code(),
        "E0901"
    );
    assert_eq!(
        err_of(r#"extern "C" { fn f(): Map<string, int> }"#).code(),
        "E0901"
    );
}

#[test]
fn e0901_on_export_signatures() {
    assert_eq!(
        err_of(r#"pub extern "C" fn f(s: string): i32 { 0 }"#).code(),
        "E0901"
    );
    assert_eq!(
        err_of(r#"pub extern "C" fn f(x: unit): i32 { 0 }"#).code(),
        "E0901"
    );
}

#[test]
fn e0230_still_governs_admitted_heads() {
    // `Ptr` is admitted, so a type application on it is the ordinary
    // non-generic rule (spec ffi.md), not E0901.
    assert_eq!(
        err_of(r#"extern "C" { fn f(x: Ptr<u8>): i32 }"#).code(),
        "E0230"
    );
}

// ---------------------------------------------------------------
// E0902 — ABI registry
// ---------------------------------------------------------------

#[test]
fn e0902_unregistered_abi_on_import_block() {
    let err = err_of(r#"extern "c++" { fn f(): i32 }"#);
    assert_eq!(err.code(), "E0902");
    assert!(
        err.message.contains("ABI `c++` is not registered"),
        "{}",
        err.message
    );
    assert_eq!(
        err.note.as_deref(),
        Some("registered ABIs: `\"C\"`, `\"system\"`")
    );
}

#[test]
fn e0902_unregistered_abi_on_export() {
    let err = err_of(r#"pub extern "rust" fn f(): i32 { 0 }"#);
    assert_eq!(err.code(), "E0902");
    assert!(err.message.contains("ABI `rust` is not registered"));
}

// ---------------------------------------------------------------
// E0903 — imports are callable in direct call position only
// ---------------------------------------------------------------

#[test]
fn e0903_binding_an_import() {
    let err = err_of(
        r#"
        extern "C" { fn abs(x: i32): i32 }
        fn main() { let g = abs }
    "#,
    );
    assert_eq!(err.code(), "E0903");
    assert!(
        err.message
            .contains("`abs` is an extern import and can only be called directly"),
        "{}",
        err.message
    );
}

#[test]
fn e0903_binding_an_aliased_import() {
    // The alias binds the import, so it is equally callable-in-place only —
    // matched against the local binding's declaration span, not the symbol's.
    let err = err_of(
        r#"
        extern "C" { fn abs(x: i32): i32 as c_abs }
        fn main() { let g = c_abs }
    "#,
    );
    assert_eq!(err.code(), "E0903");
    assert!(
        err.message
            .contains("`c_abs` is an extern import and can only be called directly"),
        "{}",
        err.message
    );
}

#[test]
fn e0903_passing_an_import_as_an_argument() {
    let err = err_of(
        r#"
        extern "C" { fn abs(x: i32): i32 }
        fn apply(f: fn(i32): i32, x: i32): i32 { f(x) }
        fn main() { print(apply(abs, -1)) }
    "#,
    );
    assert_eq!(err.code(), "E0903");
}

#[test]
fn e0903_returning_or_comparing_an_import() {
    assert_eq!(
        err_of(
            r#"
            extern "C" { fn abs(x: i32): i32 }
            fn get(): fn(i32): i32 { abs }
        "#
        )
        .code(),
        "E0903"
    );
}

// ---------------------------------------------------------------
// Registration, naming, visibility
// ---------------------------------------------------------------

#[test]
fn import_registers_in_source_order() {
    // Forward references do not work, exactly as for `fn`s.
    let err = err_of(
        r#"
        fn main() { print(abs(-1)) }
        extern "C" { fn abs(x: i32): i32 }
    "#,
    );
    assert!(
        err.message.contains("unknown function `abs`") || err.message.contains("undefined"),
        "{}",
        err.message
    );
}

#[test]
fn as_alias_binds_only_the_local_name() {
    ok(r#"
        extern "C" { fn log_message(msg: i32) as puts }
        fn main() { puts(1) }
    "#);
    let err = err_of(
        r#"
        extern "C" { fn log_message(msg: i32) as puts }
        fn main() { log_message(1) }
    "#,
    );
    assert!(
        err.message.contains("unknown function `log_message`"),
        "{}",
        err.message
    );
}

#[test]
fn duplicate_registration_is_rejected() {
    let err = err_of(
        r#"
        extern "C" { fn abs(x: i32): i32 }
        fn abs(x: i32): i32 { x }
    "#,
    );
    assert!(err.message.contains("already defined"), "{}", err.message);
}

#[test]
fn pub_block_exports_every_item() {
    let program = parse_program(
        r#"
        pub extern "C" { fn getpid(): i32 fn abort(): unit }
        extern "C" { fn hidden(): i32 }
    "#,
    )
    .unwrap();
    let result = xulo_semantic::analyze_with(&program, &[], &[], &[])
        .unwrap_or_else(|e| panic!("{}", e.message));
    let names: Vec<&str> = result
        .exported_symbols
        .iter()
        .map(|(n, _)| n.as_str())
        .collect();
    assert!(names.contains(&"getpid"), "{names:?}");
    assert!(names.contains(&"abort"), "{names:?}");
    assert!(!names.contains(&"hidden"), "{names:?}");
}

// ---------------------------------------------------------------
// Fixed-bit literals: coercion, E0209, E0201
// ---------------------------------------------------------------

#[test]
fn in_range_literals_coerce_to_fixed_bit_targets() {
    ok(r#"
        fn f(): i32 { let x: i32 = 8 x }
        fn g(): u8 { let x: u8 = 255 x }
        fn h(): usize { let n: usize = 8 n }
        fn i(): f32 { let o: f32 = 0.8 o }
        fn j(): u8 { let x: u8 = 8 x }
    "#);
    ok(r#"
        extern "C" { fn take8(x: u8): u8 }
        fn main() { take8(200) }
    "#);
}

#[test]
fn e0209_out_of_range_binding() {
    let err = err_of(r#"fn f() { let b: u8 = 300 }"#);
    assert_eq!(err.code(), "E0209");
    assert!(
        err.message.contains("literal `300` does not fit `u8`"),
        "{}",
        err.message
    );
}

#[test]
fn e0209_out_of_range_return_and_argument_and_trailing() {
    assert_eq!(err_of(r#"fn f(): u8 { return 300 }"#).code(), "E0209");
    assert_eq!(err_of(r#"fn f(): u8 { 256 }"#).code(), "E0209");
    assert_eq!(
        err_of(
            r#"
            extern "C" { fn take8(x: u8): u8 }
            fn main() { take8(999) }
        "#
        )
        .code(),
        "E0209"
    );
    assert_eq!(
        err_of(r#"fn f(): f32 { 340282400000000000000000000000000000000 }"#).code(),
        "E0209"
    );
    assert_eq!(
        err_of(r#"fn f(): u64 { 18446744073709551616 }"#).code(),
        "E0209"
    );
}

#[test]
fn e0201_float_literal_never_reaches_an_integer_type() {
    let err = err_of(r#"fn f() { let x: u8 = 3.5 }"#);
    assert_eq!(err.code(), "E0201");
}

#[test]
fn e0201_non_literal_between_fixed_bit_types() {
    let err = err_of(
        r#"
        fn f(a: u8): u16 { let x: u16 = a x }
    "#,
    );
    assert_eq!(err.code(), "E0201");
}

// ---------------------------------------------------------------
// Fixed-bit arithmetic and comparisons
// ---------------------------------------------------------------

#[test]
fn fixed_bit_arithmetic_stays_in_its_own_type() {
    ok(r#"
        extern "C" { fn abs(x: i32): i32 }
        fn addSame(a: i32, b: i32): i32 { a + b }
        fn shiftLit(a: u32): u32 { a << 2 }
        fn mask(a: u8): u8 { a & 7 }
        fn withLiteral(a: u32): u32 { a + 8 }
        fn compareLit(a: u32): bool { a < 10 }
    "#);
    ok(r#"
        extern "C" { fn abs(x: i32): i32 }
        fn counted(xs: i32, n: i32): i32 {
          let mut total: i32 = 0
          let mut i: i32 = 0
          while i < n {
            total = total + abs(xs + i)
            i = i + 1
          }
          total
        }
    "#);
}

#[test]
fn e0201_mixing_fixed_bit_widths() {
    let err = err_of(r#"fn f(a: u8, b: u16): u8 { a + b }"#);
    assert_eq!(err.code(), "E0201");
    assert_eq!(
        err_of(r#"fn f(a: usize, b: isize): usize { a + b }"#).code(),
        "E0201"
    );
    let err = err_of(r#"fn f(a: f32, b: f64): f32 { a + b }"#);
    assert_eq!(err.code(), "E0201");
    assert_eq!(
        err_of(r#"fn f(a: u32, b: u16): bool { a < b }"#).code(),
        "E0201",
        "comparisons follow the same operand rules"
    );
}

#[test]
fn e0201_mixing_fixed_bit_with_user_facing_layer() {
    assert_eq!(
        err_of(r#"fn f(a: i32, b: number): i32 { a + b }"#).code(),
        "E0201"
    );
    assert_eq!(err_of(r#"fn f(a: i32): i32 { a + 1.5 }"#).code(), "E0201");
    // An out-of-range literal in arithmetic mixes too (it is not an
    // "in-range literal of T's default type").
    assert_eq!(err_of(r#"fn f(a: u8): u8 { a + 300 }"#).code(), "E0201");
}

// ---------------------------------------------------------------
// Ptr operator restrictions
// ---------------------------------------------------------------

#[test]
fn ptr_only_supports_equality() {
    ok(r#"
        extern "C" { fn load(p: Ptr): Ptr }
        fn same(a: Ptr, b: Ptr): bool { a == b }
        fn diff(a: Ptr, b: Ptr): bool { a != b }
    "#);
    let err = err_of(
        r#"
        fn order(a: Ptr, b: Ptr): bool { a < b }
    "#,
    );
    assert!(
        err.message
            .contains("the only operators on `Ptr` are `==` and `!=`"),
        "{}",
        err.message
    );
}

#[test]
fn ptr_never_converts_to_or_from_an_integer() {
    let err = err_of(
        r#"
        fn f(a: Ptr): i32 { a + 1 }
    "#,
    );
    assert!(err.message.contains("cannot apply `+`"), "{}", err.message);
}
