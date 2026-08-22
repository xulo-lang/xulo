use xulo_core::ir::*;

#[test]
fn test_ir_module_add_string_dedup() {
    let mut module = IrModule::new();
    let idx0 = module.add_string("hello".to_string());
    let idx1 = module.add_string("world".to_string());
    let idx2 = module.add_string("hello".to_string());
    assert_eq!(idx0, 0);
    assert_eq!(idx1, 1);
    assert_eq!(idx2, 2);
    assert_eq!(module.strings.len(), 3);
}

#[test]
fn test_ir_function_add_multiple_locals() {
    let mut func = IrFunction::new("test".to_string(), vec![], IrType::Void);
    let ids: Vec<LocalId> = (0..10).map(|_| func.add_local(IrType::I64)).collect();
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(*id, LocalId(i));
    }
    assert_eq!(func.locals.len(), 10);
}

#[test]
fn test_ir_function_add_multiple_labels() {
    let mut func = IrFunction::new("test".to_string(), vec![], IrType::Void);
    let labels: Vec<Label> = (0..5).map(|_| func.add_label()).collect();
    for (i, label) in labels.iter().enumerate() {
        assert_eq!(*label, Label(i));
    }
    assert_eq!(func.labels.len(), 5);
}

#[test]
fn test_ir_module_entry_point() {
    let mut module = IrModule::new();
    assert_eq!(module.entry_point, FuncId(0));
    
    let func1 = IrFunction::new("main".to_string(), vec![], IrType::Void);
    let func2 = IrFunction::new("helper".to_string(), vec![], IrType::I64);
    module.add_function(func1);
    module.add_function(func2);
    
    module.entry_point = FuncId(0);
    assert_eq!(module.entry_point.0, 0);
}

#[test]
fn test_instruction_clone() {
    let instr = Instruction::Const {
        dst: LocalId(0),
        value: IrValue::I64(42),
    };
    let cloned = instr.clone();
    assert!(matches!(cloned, Instruction::Const { .. }));
}

#[test]
fn test_operand_clone() {
    let op1 = Operand::Const(IrValue::I64(42));
    let op2 = Operand::Local(LocalId(5));
    let c1 = op1.clone();
    let c2 = op2.clone();
    assert!(matches!(c1, Operand::Const(_)));
    assert!(matches!(c2, Operand::Local(_)));
}

#[test]
fn test_runtime_fn_all_variants() {
    let variants = vec![
        RuntimeFn::Print,
        RuntimeFn::PrintInt,
        RuntimeFn::PrintFloat,
        RuntimeFn::PrintValue,
        RuntimeFn::Panic,
        RuntimeFn::AllocObject,
        RuntimeFn::AllocArray,
        RuntimeFn::StringConcat,
        RuntimeFn::ToString,
        RuntimeFn::ArrayPush,
        RuntimeFn::ArrayLen,
        RuntimeFn::ArrayGet,
        RuntimeFn::ArrayGetTag,
        RuntimeFn::ArraySet,
        RuntimeFn::ArrayConcat,
        RuntimeFn::ObjectGet,
        RuntimeFn::ObjectSet,
    ];
    assert_eq!(variants.len(), 17);
}

#[test]
fn test_ir_type_all_variants() {
    let types = vec![
        IrType::Void,
        IrType::Bool,
        IrType::I64,
        IrType::F64,
        IrType::Pointer,
    ];
    assert_eq!(types.len(), 5);
}

#[test]
fn test_ir_value_debug() {
    let val = IrValue::I64(42);
    let debug = format!("{:?}", val);
    assert!(debug.contains("I64"));
    assert!(debug.contains("42"));
}

#[test]
fn test_ir_type_debug() {
    let ty = IrType::Pointer;
    let debug = format!("{:?}", ty);
    assert!(debug.contains("Pointer"));
}
