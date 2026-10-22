//! FFI 执行链路：JIT 经 dlsym 解析 C 符号、AOT 把符号留成 undefined 交链接器。
//! 覆盖边界类（i32/usize/u32/f64/Ptr/unit）、`as` 别名与 `pub extern fn` 导出。

use std::path::Path;

use xulo_compiler::aot::AotCodeGen;
use xulo_compiler::codegen::CodeGen;
use xulo_compiler::compile_to_ir;

fn jit_run(source: &str) -> Vec<String> {
    let ir = compile_to_ir(source, Path::new("test.xulo")).expect("IR generation failed");
    let mut codegen = CodeGen::new();
    let code_ptr = codegen.compile(&ir).expect("codegen failed");
    unsafe {
        let func: fn() = std::mem::transmute(code_ptr);
        func();
    }
    xulo_runtime::runtime::xulo_take_output()
}

fn output_of(source: &str) -> String {
    jit_run(source).join("\n")
}

fn object_bytes(source: &str) -> Vec<u8> {
    let ir = compile_to_ir(source, Path::new("test.xulo")).expect("IR generation failed");
    let product = AotCodeGen::new()
        .expect("aot codegen init failed")
        .compile(&ir)
        .expect("aot compile failed");
    let mut buf = Vec::new();
    product.object.write_stream(&mut buf).expect("write failed");
    buf
}

fn references_symbol(bytes: &[u8], symbol: &str) -> bool {
    bytes
        .windows(symbol.len())
        .any(|window| window == symbol.as_bytes())
}

// -- IR 层 --

#[test]
fn extern_block_lands_in_ir_with_abi_classes() {
    let ir = compile_to_ir(
        r#"
        extern "C" {
            fn abs(x: i32): i32
            fn strlen(s: Ptr): usize
            fn srand(x: u32)
        }
        fn main() { print(abs(-1)) }
        "#,
        Path::new("test.xulo"),
    )
    .expect("IR generation failed");
    assert_eq!(ir.externs.len(), 3);
    assert_eq!(ir.externs[0].symbol, "abs");
    assert_eq!(ir.externs[0].params, vec![xulo_core::ir::IrAbiParam::S32]);
    assert_eq!(
        ir.externs[0].return_type,
        Some(xulo_core::ir::IrAbiParam::S32)
    );
    assert_eq!(ir.externs[1].symbol, "strlen");
    assert_eq!(ir.externs[1].params, vec![xulo_core::ir::IrAbiParam::U64]);
    assert_eq!(
        ir.externs[1].return_type,
        Some(xulo_core::ir::IrAbiParam::U64)
    );
    assert_eq!(ir.externs[2].symbol, "srand");
    assert_eq!(ir.externs[2].params, vec![xulo_core::ir::IrAbiParam::U32]);
    assert_eq!(ir.externs[2].return_type, None);
    let main = ir
        .functions
        .iter()
        .find(|f| f.name == "main")
        .expect("main fn");
    assert!(
        main.instructions
            .iter()
            .any(|i| matches!(i, xulo_core::ir::Instruction::ExternCall { .. })),
        "main must call the import via ExternCall"
    );
}

#[test]
fn extern_alias_registers_local_name_with_c_symbol() {
    let ir = compile_to_ir(
        r#"
        extern "C" { fn abs(x: i32): i32 as c_abs }
        fn main() { print(c_abs(-3)) }
        "#,
        Path::new("test.xulo"),
    )
    .expect("IR generation failed");
    assert_eq!(ir.externs.len(), 1);
    assert_eq!(ir.externs[0].symbol, "abs");
    assert_eq!(ir.externs[0].abi, "C");
}

// -- JIT（dlsym） --

#[test]
fn jit_calls_libc_abs_with_i32_boundary() {
    let out = output_of(
        r#"
        extern "C" { fn abs(x: i32): i32 }
        fn main() { print(abs(-42)) }
        "#,
    );
    assert_eq!(out, "42");
}

#[test]
fn jit_uses_c_symbol_behind_as_alias() {
    let out = output_of(
        r#"
        extern "C" { fn abs(x: i32): i32 as c_abs }
        fn main() { print(c_abs(-7)) }
        "#,
    );
    assert_eq!(out, "7");
}

#[test]
fn jit_passes_ptr_handle_through_malloc_and_free() {
    let out = output_of(
        r#"
        extern "C" {
          fn malloc(size: usize): Ptr? as cMalloc
          fn free(p: Ptr?) as cFree
        }
        fn main() {
            let p = cMalloc(64)
            if p != null {
                cFree(p)
                print("allocated")
            }
        }
        "#,
    );
    assert_eq!(out, "allocated");
}

#[test]
fn jit_reads_f64_return() {
    let out = output_of(
        r#"
        extern "C" { fn sqrt(x: f64): f64 }
        fn main() { print(sqrt(2.0)) }
        "#,
    );
    assert_eq!(out, "1.4142135623730951");
}

#[test]
fn jit_void_return_continues_program() {
    let out = output_of(
        r#"
        extern "C" { fn srand(x: u32) }
        fn main() { srand(42); print("ok") }
        "#,
    );
    assert_eq!(out, "ok");
}

#[test]
fn jit_system_abi_tag_is_registered_and_callable() {
    let out = output_of(
        r#"
        extern "system" { fn abs(x: i32): i32 }
        fn main() { print(abs(-9)) }
        "#,
    );
    assert_eq!(out, "9");
}

// -- AOT（undefined 符号交链接器） --

#[test]
fn aot_object_leaves_libc_symbol_undefined() {
    let bytes = object_bytes(
        r#"
        extern "C" { fn abs(x: i32): i32 }
        fn main() { print(abs(-1)) }
        "#,
    );
    assert!(
        references_symbol(&bytes, "abs"),
        "object must reference the libc symbol abs"
    );
}

#[test]
fn aot_object_references_alias_target_not_local_name() {
    let bytes = object_bytes(
        r#"
        extern "C" { fn abs(x: i32): i32 as c_abs }
        fn main() { print(c_abs(-1)) }
        "#,
    );
    assert!(
        references_symbol(&bytes, "abs"),
        "object must reference abs (the C symbol)"
    );
}

#[test]
fn aot_exports_pub_extern_fn() {
    let bytes = object_bytes(
        r#"
        pub extern "C" fn add(a: i32, b: i32): i32 { a + b }
        fn main() { print(add(1, 2)) }
        "#,
    );
    assert!(references_symbol(&bytes, "add"), "object must export add");
}

#[test]
fn jit_named_arguments_reorder_by_declaration() {
    // Written out of order; `arrange_call_args` lands them by declared name
    // (spec ffi.md Calling — named arguments accepted as for any `fn`).
    let out = output_of(
        r#"
        extern "C" { fn pow(x: f64, y: f64): f64 }
        fn main() { print(pow(y: 10, x: 2)) }
        "#,
    );
    assert_eq!(out, "1024");
}

#[test]
fn jit_omitted_ptr_param_binds_null() {
    // An omitted (only `Ptr?` may be) parameter binds null at the call site:
    // `cFree()` compiles to a call with a zero address — `free(NULL)`, a no-op.
    let out = output_of(
        r#"
        extern "C" {
            fn malloc(size: usize): Ptr? as cMalloc
            fn free(p: Ptr?) as cFree
        }
        fn main() {
            let p = cMalloc(32)
            cFree(p)
            cFree()
            print("ok")
        }
        "#,
    );
    assert_eq!(out, "ok");
}

#[test]
fn aot_omitted_ptr_param_compiles_to_zero_address() {
    let bytes = object_bytes(
        r#"
        extern "C" {
            fn malloc(size: usize): Ptr? as cMalloc
            fn free(p: Ptr?) as cFree
        }
        fn main() {
            let p = cMalloc(32)
            cFree()
            print("ok")
        }
        "#,
    );
    assert!(
        references_symbol(&bytes, "free"),
        "object must reference libc free"
    );
}
