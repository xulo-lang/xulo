//! IR/ABI 类型到 LLVM 类型的映射。
//!
//! 用户函数与运行时函数沿用 Cranelift 后端的约定：IR 值槽位永远是 `i64`
//! （`F64` 局部变量用 `double`），FFI 边界才出现窄整数与 `f32`。

use inkwell::context::Context;
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum, FunctionType};

use xulo_core::ir::{IrAbiParam, IrType};

/// IR 局部变量/参数槽位类型。除 `F64` 外都占一个 `i64` 槽。
pub fn ir_slot_type<'ctx>(context: &'ctx Context, ty: &IrType) -> BasicTypeEnum<'ctx> {
    match ty {
        IrType::F64 => context.f64_type().into(),
        _ => context.i64_type().into(),
    }
}

/// FFI 边界类 -> C ABI 类型（与 Cranelift 的 `abi_clift_type` 对齐）。
pub fn abi_type<'ctx>(context: &'ctx Context, p: IrAbiParam) -> BasicTypeEnum<'ctx> {
    match p {
        IrAbiParam::Bool | IrAbiParam::S8 | IrAbiParam::U8 => context.i8_type().into(),
        IrAbiParam::S16 | IrAbiParam::U16 => context.i16_type().into(),
        IrAbiParam::S32 | IrAbiParam::U32 => context.i32_type().into(),
        IrAbiParam::S64 | IrAbiParam::U64 => context.i64_type().into(),
        IrAbiParam::F32 => context.f32_type().into(),
        IrAbiParam::F64 => context.f64_type().into(),
    }
}

/// `RUNTIME_FNS` 里的 Cranelift 类型 -> LLVM 类型。
pub fn runtime_type<'ctx>(
    context: &'ctx Context,
    ty: cranelift::codegen::ir::Type,
) -> BasicTypeEnum<'ctx> {
    use cranelift::codegen::ir::types;
    if ty == types::F64 {
        context.f64_type().into()
    } else if ty == types::F32 {
        context.f32_type().into()
    } else if ty == types::I8 {
        context.i8_type().into()
    } else if ty == types::I16 {
        context.i16_type().into()
    } else if ty == types::I32 {
        context.i32_type().into()
    } else {
        context.i64_type().into()
    }
}

/// 由参数/返回类型构造 LLVM 函数类型。
pub fn function_type<'ctx>(
    context: &'ctx Context,
    params: &[BasicTypeEnum<'ctx>],
    returns: Option<BasicTypeEnum<'ctx>>,
) -> FunctionType<'ctx> {
    let param_tys: Vec<BasicMetadataTypeEnum<'ctx>> = params.iter().map(|t| (*t).into()).collect();
    match returns {
        Some(BasicTypeEnum::IntType(ty)) => ty.fn_type(&param_tys, false),
        Some(BasicTypeEnum::FloatType(ty)) => ty.fn_type(&param_tys, false),
        Some(BasicTypeEnum::PointerType(ty)) => ty.fn_type(&param_tys, false),
        Some(_) => context.void_type().fn_type(&param_tys, false),
        None => context.void_type().fn_type(&param_tys, false),
    }
}
