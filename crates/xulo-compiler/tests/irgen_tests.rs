use std::path::Path;
use xulo_compiler::compile_to_ir;
use xulo_core::ir::*;

fn compile_to_ir_helper(source: &str) -> IrModule {
    compile_to_ir(source, Path::new("test.xulo")).expect("failed to compile to IR")
}

#[test]
fn test_compile_hello_world() {
    let module = compile_to_ir_helper(r#"fn main() { print("Hello") }"#);
    assert_eq!(module.functions.len(), 1);
    assert_eq!(module.functions[0].name, "main");
    assert!(!module.strings.is_empty());
}

#[test]
fn test_compile_empty_main() {
    let module = compile_to_ir_helper("fn main() {}");
    assert_eq!(module.functions.len(), 1);
    assert_eq!(module.functions[0].name, "main");
    // 空函数体也会生成 Return(None)
    assert!(module.functions[0].instructions.iter().any(|i| matches!(i, Instruction::Return(None))));
}

#[test]
fn test_compile_let_binding() {
    let module = compile_to_ir_helper(r#"fn main() { let x = 42 }"#);
    let func = &module.functions[0];
    assert!(!func.locals.is_empty());
    assert!(func.instructions.iter().any(|i| matches!(i, Instruction::SetLocal { .. })));
}

#[test]
fn test_compile_arithmetic() {
    let module = compile_to_ir_helper(r#"fn main() { let x = 1 + 2 }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(i, Instruction::Add { .. })));
}

#[test]
fn test_compile_subtraction() {
    let module = compile_to_ir_helper(r#"fn main() { let x = 10 - 3 }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(i, Instruction::Sub { .. })));
}

#[test]
fn test_compile_multiplication() {
    let module = compile_to_ir_helper(r#"fn main() { let x = 4 * 5 }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(i, Instruction::Mul { .. })));
}

#[test]
fn test_compile_division() {
    let module = compile_to_ir_helper(r#"fn main() { let x = 20 / 4 }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(i, Instruction::Div { .. })));
}

#[test]
fn test_compile_comparison() {
    let module = compile_to_ir_helper(r#"fn main() { let x = 1 < 2 }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(i, Instruction::Lt { .. })));
}

#[test]
fn test_compile_equality() {
    let module = compile_to_ir_helper(r#"fn main() { let x = 1 == 1 }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(i, Instruction::Eq { .. })));
}

#[test]
fn test_compile_if_else() {
    let source = r#"
fn main() {
    let x = 10
    if x > 5 {
        print("big")
    } else {
        print("small")
    }
}
"#;
    let module = compile_to_ir_helper(source);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(i, Instruction::Branch { .. })));
    assert!(func.instructions.iter().any(|i| matches!(i, Instruction::Label(_))));
}

#[test]
fn test_compile_while_loop() {
    let source = r#"
fn main() {
    let mut i = 0
    while i < 10 {
        i = i + 1
    }
}
"#;
    let module = compile_to_ir_helper(source);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(i, Instruction::Branch { .. })));
}

#[test]
fn test_compile_for_loop() {
    let source = r#"
fn main() {
    let items = [1, 2, 3]
    for item in items {
        print(item)
    }
}
"#;
    let module = compile_to_ir_helper(source);
    let func = &module.functions[0];
    // for 循环生成 Label、Jump、RuntimeCall 等指令
    assert!(func.instructions.len() > 3);
    assert!(!func.labels.is_empty());
}

#[test]
fn test_compile_function_call() {
    let source = r#"
fn add(a, b) {
    return a + b
}

fn main() {
    let result = add(1, 2)
}
"#;
    let module = compile_to_ir_helper(source);
    assert!(module.functions.len() >= 2);
}

#[test]
fn test_compile_string_literal() {
    let module = compile_to_ir_helper(r#"fn main() { print("hello world") }"#);
    assert!(!module.strings.is_empty());
    assert!(module.strings.iter().any(|s| s == "hello world"));
}

#[test]
fn test_compile_multiple_strings() {
    let source = r#"
fn main() {
    print("first")
    print("second")
}
"#;
    let module = compile_to_ir_helper(source);
    assert!(module.strings.len() >= 2);
}

#[test]
fn test_compile_negative_number() {
    let module = compile_to_ir_helper(r#"fn main() { let x = -5 }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(i, Instruction::Neg { .. })));
}

#[test]
fn test_compile_boolean_literal() {
    let module = compile_to_ir_helper(r#"fn main() { let x = true }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(i, Instruction::Const { value: IrValue::Bool(true), .. })));
}

#[test]
fn test_compile_null_literal() {
    let module = compile_to_ir_helper(r#"fn main() { let x = null }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(i, Instruction::Const { value: IrValue::Null, .. })));
}

#[test]
fn test_compile_print_int() {
    let module = compile_to_ir_helper(r#"fn main() { print(42) }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(
        i, Instruction::RuntimeCall { func: RuntimeFn::PrintValue, .. }
    )));
}

#[test]
fn test_compile_print_string() {
    let module = compile_to_ir_helper(r#"fn main() { print("hi") }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(
        i, Instruction::RuntimeCall { func: RuntimeFn::PrintValue, .. }
    )));
}

#[test]
fn test_compile_entry_point() {
    let module = compile_to_ir_helper(r#"fn main() { print("hello") }"#);
    assert_eq!(module.entry_point.0, 0);
    assert_eq!(module.functions[module.entry_point.0].name, "main");
}

// -- new instruction coverage --

#[test]
fn test_compile_new_array_with_tag() {
    let module = compile_to_ir_helper(r#"fn main() { let arr = [1, 2, 3] }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(i, Instruction::NewArray { .. })));
}

#[test]
fn test_compile_new_object_with_tag() {
    let module = compile_to_ir_helper(r#"fn main() { let obj = {a: 1, b: 2} }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(i, Instruction::NewObject { .. })));
}

#[test]
fn test_compile_array_concat_generates_runtime_call() {
    let module = compile_to_ir_helper(r#"fn main() { print([1] + [2]) }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(
        i, Instruction::RuntimeCall { func: RuntimeFn::ArrayConcat, .. }
    )));
}

#[test]
fn test_compile_print_value_for_int() {
    let module = compile_to_ir_helper(r#"fn main() { print(42) }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(
        i, Instruction::RuntimeCall { func: RuntimeFn::PrintValue, .. }
    )));
}

#[test]
fn test_compile_print_value_for_string() {
    let module = compile_to_ir_helper(r#"fn main() { print("hi") }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(
        i, Instruction::RuntimeCall { func: RuntimeFn::PrintValue, .. }
    )));
}

#[test]
fn test_compile_print_value_for_bool() {
    let module = compile_to_ir_helper(r#"fn main() { print(true) }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(
        i, Instruction::RuntimeCall { func: RuntimeFn::PrintValue, .. }
    )));
}

#[test]
fn test_compile_print_value_for_array() {
    let module = compile_to_ir_helper(r#"fn main() { print([1, 2]) }"#);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(
        i, Instruction::RuntimeCall { func: RuntimeFn::PrintValue, .. }
    )));
}

#[test]
fn test_compile_for_loop_type_inference() {
    let source = r#"
fn main() {
    for x in [1, 2, 3] {
        print(x)
    }
}
"#;
    let module = compile_to_ir_helper(source);
    let func = &module.functions[0];
    // for 循环应该生成 GetIndex 指令来迭代
    assert!(func.instructions.iter().any(|i| matches!(i, Instruction::GetIndex { .. })));
}

#[test]
fn test_compile_generic_function_call() {
    let source = r#"
fn identity<T>(value: T): T {
    value
}
fn main() {
    print(identity(42))
}
"#;
    let module = compile_to_ir_helper(source);
    assert!(module.functions.len() >= 2);
    // main 应该生成 Call 指令
    let main_func = module.functions.iter().find(|f| f.name == "main").unwrap();
    assert!(main_func.instructions.iter().any(|i| matches!(i, Instruction::Call { .. })));
}

#[test]
fn test_compile_array_concat_variable() {
    let source = r#"
fn main() {
    let a = [1, 2]
    let b = [3, 4]
    print(a + b)
}
"#;
    let module = compile_to_ir_helper(source);
    let func = &module.functions[0];
    assert!(func.instructions.iter().any(|i| matches!(
        i, Instruction::RuntimeCall { func: RuntimeFn::ArrayConcat, .. }
    )));
}

#[test]
fn test_compile_array_get_tag() {
    let source = r#"
fn main() {
    for x in [1, 2, 3] {
        print(x)
    }
}
"#;
    let module = compile_to_ir_helper(source);
    let func = &module.functions[0];
    // for 循环对 list 应该生成 ArrayLen
    assert!(func.instructions.iter().any(|i| matches!(
        i, Instruction::RuntimeCall { func: RuntimeFn::ArrayLen, .. }
    )));
}
