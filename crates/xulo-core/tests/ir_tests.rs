use xulo_core::ir::*;
use xulo_core::error::{XuloError, ErrorKind};

// ========== IrType tests ==========

#[test]
fn test_ir_type_is_void() {
    assert!(IrType::Void.is_void());
    assert!(!IrType::Bool.is_void());
    assert!(!IrType::I64.is_void());
    assert!(!IrType::F64.is_void());
    assert!(!IrType::Pointer.is_void());
}

#[test]
fn test_ir_type_display() {
    assert_eq!(IrType::Void.to_string(), "void");
    assert_eq!(IrType::Bool.to_string(), "bool");
    assert_eq!(IrType::I64.to_string(), "i64");
    assert_eq!(IrType::F64.to_string(), "f64");
    assert_eq!(IrType::Pointer.to_string(), "ptr");
}

// ========== IrValue tests ==========

#[test]
fn test_ir_value_display() {
    assert_eq!(IrValue::Bool(true).to_string(), "true");
    assert_eq!(IrValue::Bool(false).to_string(), "false");
    assert_eq!(IrValue::I64(42).to_string(), "42");
    assert_eq!(IrValue::I64(-1).to_string(), "-1");
    assert_eq!(IrValue::F64(3.14).to_string(), "3.14");
    assert_eq!(IrValue::String(0).to_string(), "str[0]");
    assert_eq!(IrValue::String(5).to_string(), "str[5]");
    assert_eq!(IrValue::Null.to_string(), "null");
}

// ========== IrModule tests ==========

#[test]
fn test_ir_module_new() {
    let module = IrModule::new();
    assert!(module.functions.is_empty());
    assert!(module.strings.is_empty());
}

#[test]
fn test_ir_module_add_function() {
    let mut module = IrModule::new();
    let func = IrFunction::new("main".to_string(), vec![], IrType::Void);
    let id = module.add_function(func);
    assert_eq!(id, FuncId(0));
    assert_eq!(module.functions.len(), 1);
    assert_eq!(module.functions[0].name, "main");
}

#[test]
fn test_ir_module_add_multiple_functions() {
    let mut module = IrModule::new();
    let func1 = IrFunction::new("main".to_string(), vec![], IrType::Void);
    let func2 = IrFunction::new("helper".to_string(), vec![], IrType::I64);
    let id1 = module.add_function(func1);
    let id2 = module.add_function(func2);
    assert_eq!(id1, FuncId(0));
    assert_eq!(id2, FuncId(1));
    assert_eq!(module.functions.len(), 2);
}

#[test]
fn test_ir_module_add_string() {
    let mut module = IrModule::new();
    let idx0 = module.add_string("hello".to_string());
    let idx1 = module.add_string("world".to_string());
    assert_eq!(idx0, 0);
    assert_eq!(idx1, 1);
    assert_eq!(module.strings.len(), 2);
    assert_eq!(module.strings[0], "hello");
    assert_eq!(module.strings[1], "world");
}

// ========== IrFunction tests ==========

#[test]
fn test_ir_function_new() {
    let params = vec![
        (LocalId(0), IrType::I64),
        (LocalId(1), IrType::Bool),
    ];
    let func = IrFunction::new("add".to_string(), params.clone(), IrType::I64);
    assert_eq!(func.name, "add");
    assert_eq!(func.params, params);
    assert_eq!(func.return_type, IrType::I64);
    assert!(func.locals.is_empty());
    assert!(func.instructions.is_empty());
    assert!(func.labels.is_empty());
}

#[test]
fn test_ir_function_add_local() {
    let mut func = IrFunction::new("test".to_string(), vec![], IrType::Void);
    let id0 = func.add_local(IrType::I64);
    let id1 = func.add_local(IrType::Bool);
    let id2 = func.add_local(IrType::Pointer);
    assert_eq!(id0, LocalId(0));
    assert_eq!(id1, LocalId(1));
    assert_eq!(id2, LocalId(2));
    assert_eq!(func.locals.len(), 3);
    assert_eq!(func.locals[0].1, IrType::I64);
    assert_eq!(func.locals[1].1, IrType::Bool);
    assert_eq!(func.locals[2].1, IrType::Pointer);
}

#[test]
fn test_ir_function_add_label() {
    let mut func = IrFunction::new("test".to_string(), vec![], IrType::Void);
    let label0 = func.add_label();
    let label1 = func.add_label();
    let label2 = func.add_label();
    assert_eq!(label0, Label(0));
    assert_eq!(label1, Label(1));
    assert_eq!(label2, Label(2));
    assert_eq!(func.labels.len(), 3);
}

// ========== Instruction tests ==========

#[test]
fn test_instruction_const() {
    let instr = Instruction::Const {
        dst: LocalId(0),
        value: IrValue::I64(42),
    };
    assert!(matches!(instr, Instruction::Const { .. }));
}

#[test]
fn test_instruction_arithmetic() {
    let left = Operand::Local(LocalId(0));
    let right = Operand::Const(IrValue::I64(10));
    
    let add = Instruction::Add { dst: LocalId(1), left: left.clone(), right: right.clone() };
    let sub = Instruction::Sub { dst: LocalId(1), left: left.clone(), right: right.clone() };
    let mul = Instruction::Mul { dst: LocalId(1), left: left.clone(), right: right.clone() };
    let div = Instruction::Div { dst: LocalId(1), left: left.clone(), right: right.clone() };
    let neg = Instruction::Neg { dst: LocalId(1), operand: left.clone() };
    
    assert!(matches!(add, Instruction::Add { .. }));
    assert!(matches!(sub, Instruction::Sub { .. }));
    assert!(matches!(mul, Instruction::Mul { .. }));
    assert!(matches!(div, Instruction::Div { .. }));
    assert!(matches!(neg, Instruction::Neg { .. }));
}

#[test]
fn test_instruction_comparison() {
    let left = Operand::Local(LocalId(0));
    let right = Operand::Const(IrValue::I64(10));
    
    let eq = Instruction::Eq { dst: LocalId(1), left: left.clone(), right: right.clone() };
    let neq = Instruction::Neq { dst: LocalId(1), left: left.clone(), right: right.clone() };
    let lt = Instruction::Lt { dst: LocalId(1), left: left.clone(), right: right.clone() };
    let gt = Instruction::Gt { dst: LocalId(1), left: left.clone(), right: right.clone() };
    let lte = Instruction::Lte { dst: LocalId(1), left: left.clone(), right: right.clone() };
    let gte = Instruction::Gte { dst: LocalId(1), left: left.clone(), right: right.clone() };
    
    assert!(matches!(eq, Instruction::Eq { .. }));
    assert!(matches!(neq, Instruction::Neq { .. }));
    assert!(matches!(lt, Instruction::Lt { .. }));
    assert!(matches!(gt, Instruction::Gt { .. }));
    assert!(matches!(lte, Instruction::Lte { .. }));
    assert!(matches!(gte, Instruction::Gte { .. }));
}

#[test]
fn test_instruction_logic() {
    let left = Operand::Local(LocalId(0));
    let right = Operand::Const(IrValue::Bool(true));
    
    let and = Instruction::And { dst: LocalId(1), left: left.clone(), right: right.clone() };
    let or = Instruction::Or { dst: LocalId(1), left: left.clone(), right: right.clone() };
    let not = Instruction::Not { dst: LocalId(1), operand: left.clone() };
    
    assert!(matches!(and, Instruction::And { .. }));
    assert!(matches!(or, Instruction::Or { .. }));
    assert!(matches!(not, Instruction::Not { .. }));
}

#[test]
fn test_instruction_control_flow() {
    let branch = Instruction::Branch {
        condition: Operand::Local(LocalId(0)),
        then_label: Label(0),
        else_label: Label(1),
    };
    let jump = Instruction::Jump(Label(0));
    let return_none = Instruction::Return(None);
    let return_some = Instruction::Return(Some(Operand::Const(IrValue::I64(42))));
    
    assert!(matches!(branch, Instruction::Branch { .. }));
    assert!(matches!(jump, Instruction::Jump(_)));
    assert!(matches!(return_none, Instruction::Return(None)));
    assert!(matches!(return_some, Instruction::Return(Some(_))));
}

#[test]
fn test_instruction_runtime_call() {
    let call = Instruction::RuntimeCall {
        dst: Some(LocalId(0)),
        func: RuntimeFn::Print,
        args: vec![Operand::Local(LocalId(1))],
    };
    assert!(matches!(call, Instruction::RuntimeCall { func: RuntimeFn::Print, .. }));
}

#[test]
fn test_instruction_set_get_local() {
    let set = Instruction::SetLocal {
        dst: LocalId(0),
        src: Operand::Const(IrValue::I64(42)),
    };
    let get = Instruction::GetLocal {
        dst: LocalId(0),
        src: LocalId(1),
    };
    assert!(matches!(set, Instruction::SetLocal { .. }));
    assert!(matches!(get, Instruction::GetLocal { .. }));
}

// ========== RuntimeFn tests ==========

#[test]
fn test_runtime_fn_equality() {
    assert_eq!(RuntimeFn::Print, RuntimeFn::Print);
    assert_eq!(RuntimeFn::PrintInt, RuntimeFn::PrintInt);
    assert_eq!(RuntimeFn::PrintFloat, RuntimeFn::PrintFloat);
    assert_eq!(RuntimeFn::Panic, RuntimeFn::Panic);
    assert_eq!(RuntimeFn::AllocObject, RuntimeFn::AllocObject);
    assert_eq!(RuntimeFn::AllocArray, RuntimeFn::AllocArray);
    assert_eq!(RuntimeFn::StringConcat, RuntimeFn::StringConcat);
    assert_eq!(RuntimeFn::ToString, RuntimeFn::ToString);
    assert_eq!(RuntimeFn::ArrayPush, RuntimeFn::ArrayPush);
    assert_eq!(RuntimeFn::ArrayLen, RuntimeFn::ArrayLen);
    assert_eq!(RuntimeFn::ArrayGet, RuntimeFn::ArrayGet);
    assert_eq!(RuntimeFn::ArraySet, RuntimeFn::ArraySet);
    assert_eq!(RuntimeFn::ObjectGet, RuntimeFn::ObjectGet);
    assert_eq!(RuntimeFn::ObjectSet, RuntimeFn::ObjectSet);
    
    assert_ne!(RuntimeFn::Print, RuntimeFn::PrintInt);
    assert_ne!(RuntimeFn::Panic, RuntimeFn::ToString);
}

// ========== Error tests ==========

#[test]
fn test_xulo_error_new() {
    let err = XuloError::new(ErrorKind::Parse, "syntax error");
    assert_eq!(err.kind, ErrorKind::Parse);
    assert_eq!(err.message, "syntax error");
    assert!(err.span.is_none());
    assert!(err.file.is_none());
}

#[test]
fn test_xulo_error_at() {
    let err = XuloError::new(ErrorKind::Lex, "unexpected character")
        .at(5..10);
    assert_eq!(err.span, Some(5..10));
}

#[test]
fn test_xulo_error_with_file() {
    use std::path::PathBuf;
    let err = XuloError::new(ErrorKind::Io, "file not found")
        .with_file(PathBuf::from("test.xulo"));
    assert_eq!(err.file, Some(PathBuf::from("test.xulo")));
}

#[test]
fn test_xulo_error_with_message_prefix() {
    let err = XuloError::new(ErrorKind::Semantic, "undefined variable")
        .with_message_prefix("in function 'main': ");
    assert_eq!(err.message, "in function 'main': undefined variable");
}

#[test]
fn test_xulo_error_display() {
    let err = XuloError::new(ErrorKind::Codegen, "compilation failed");
    assert_eq!(err.to_string(), "code generation: compilation failed");
}

#[test]
fn test_xulo_error_kind() {
    let err = XuloError::new(ErrorKind::Runtime, "runtime error");
    assert_eq!(err.kind(), ErrorKind::Runtime);
}

#[test]
fn test_xulo_error_from_io_error() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let err: XuloError = io_err.into();
    assert_eq!(err.kind, ErrorKind::Io);
    assert!(err.message.contains("file not found"));
}

#[test]
fn test_xulo_error_is_std_error() {
    let err = XuloError::new(ErrorKind::Parse, "test error");
    let _: &dyn std::error::Error = &err;
}

// ========== Integration tests ==========

#[test]
fn test_build_simple_function() {
    let mut module = IrModule::new();
    
    // Create a simple main function that returns 42
    let mut func = IrFunction::new("main".to_string(), vec![], IrType::I64);
    let local = func.add_local(IrType::I64);
    func.instructions.push(Instruction::Const {
        dst: local,
        value: IrValue::I64(42),
    });
    func.instructions.push(Instruction::Return(Some(Operand::Local(local))));
    
    let func_id = module.add_function(func);
    module.entry_point = func_id;
    
    assert_eq!(module.functions.len(), 1);
    assert_eq!(module.functions[0].instructions.len(), 2);
}

#[test]
fn test_build_function_with_params() {
    let mut module = IrModule::new();
    
    let params = vec![
        (LocalId(0), IrType::I64),
        (LocalId(1), IrType::I64),
    ];
    let mut func = IrFunction::new("add".to_string(), params, IrType::I64);
    
    let result = func.add_local(IrType::I64);
    func.instructions.push(Instruction::Add {
        dst: result,
        left: Operand::Local(LocalId(0)),
        right: Operand::Local(LocalId(1)),
    });
    func.instructions.push(Instruction::Return(Some(Operand::Local(result))));
    
    module.add_function(func);
    
    assert_eq!(module.functions[0].params.len(), 2);
    assert_eq!(module.functions[0].locals.len(), 1);
    assert_eq!(module.functions[0].instructions.len(), 2);
}

#[test]
fn test_build_function_with_control_flow() {
    let mut module = IrModule::new();
    
    let mut func = IrFunction::new("if_else".to_string(), vec![], IrType::I64);
    
    let cond = func.add_local(IrType::Bool);
    let result = func.add_local(IrType::I64);
    let then_label = func.add_label();
    let else_label = func.add_label();
    let end_label = func.add_label();
    
    // if (true) { result = 1 } else { result = 0 }
    func.instructions.push(Instruction::Const {
        dst: cond,
        value: IrValue::Bool(true),
    });
    func.instructions.push(Instruction::Branch {
        condition: Operand::Local(cond),
        then_label,
        else_label,
    });
    func.instructions.push(Instruction::Label(then_label));
    func.instructions.push(Instruction::Const {
        dst: result,
        value: IrValue::I64(1),
    });
    func.instructions.push(Instruction::Jump(end_label));
    func.instructions.push(Instruction::Label(else_label));
    func.instructions.push(Instruction::Const {
        dst: result,
        value: IrValue::I64(0),
    });
    func.instructions.push(Instruction::Label(end_label));
    func.instructions.push(Instruction::Return(Some(Operand::Local(result))));
    
    module.add_function(func);
    
    assert_eq!(module.functions[0].labels.len(), 3);
    assert_eq!(module.functions[0].instructions.len(), 9);
}
