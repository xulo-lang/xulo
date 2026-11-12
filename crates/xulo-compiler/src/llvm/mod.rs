//! `IrModule` -> LLVM IR lowering（AOT 专用；解释器与 Cranelift JIT 不受影响）。
//!
//! 指令逐条对照 `codegen.rs`（Cranelift）实现，保证两个后端语义一致：
//! 值槽位一律 `i64`（`F64` 局部用 `double`），运行时函数按 `RUNTIME_FNS`
//! 声明，闭包入口固定 `(i64 × 9) -> i64`。
//!
//! 模块划分：`function`（函数体与收尾）、`instructions`（指令分派）、
//! `values`（槽位/转换/字符串）、`numeric`（数值、比较、位运算与 trap）、
//! `calls`（运行时/FFI/闭包调用）、`collections`（对象/数组），
//! `abi` 与 `object` 负责类型映射和目标文件输出。

use std::collections::HashMap;
use std::path::Path;

use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, FloatValue, FunctionValue, IntValue, PointerValue,
};
use inkwell::{FloatPredicate, IntPredicate};

use xulo_core::error::{ErrorKind, XuloError};
use xulo_core::ir::{IrAbiParam, IrFunction, IrModule, IrType, Label, LocalId, Operand};

use crate::backend::RUNTIME_FNS;
use crate::codegen::aot_symbol_name;

pub mod abi;
pub mod object;

mod calls;
mod collections;
mod function;
mod instructions;
mod numeric;
mod values;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotKind {
    I64,
    F64,
}

impl SlotKind {
    fn of(ty: &IrType) -> Self {
        match ty {
            IrType::F64 => SlotKind::F64,
            _ => SlotKind::I64,
        }
    }
}

#[derive(Clone, Copy)]
struct Slot<'ctx> {
    ptr: PointerValue<'ctx>,
    kind: SlotKind,
}

#[derive(Clone, Copy)]
enum RuntimeKind {
    I64,
    F64,
}

#[derive(Clone)]
struct RuntimeSig<'ctx> {
    fv: FunctionValue<'ctx>,
    params: Vec<RuntimeKind>,
    ret: Option<RuntimeKind>,
}

fn codegen_err(msg: impl std::fmt::Display) -> XuloError {
    XuloError::new(ErrorKind::Codegen, msg.to_string())
}

fn cg<T>(result: Result<T, inkwell::builder::BuilderError>) -> Result<T, XuloError> {
    result.map_err(codegen_err)
}

fn runtime_kind(ty: cranelift::codegen::ir::Type) -> RuntimeKind {
    if ty == cranelift::codegen::ir::types::F64 {
        RuntimeKind::F64
    } else {
        RuntimeKind::I64
    }
}

/// 把一个 `IrModule` 降到 LLVM 模块（不落盘；`object` 负责目标文件输出）。
pub fn build_module<'ctx>(
    context: &'ctx Context,
    ir: &IrModule,
) -> Result<Module<'ctx>, XuloError> {
    let mut emitter = Emitter::new(context, ir);
    emitter.declare_runtime_functions()?;
    emitter.declare_externs()?;
    emitter.declare_functions()?;
    emitter.lower_functions()?;

    if let Err(errors) = emitter.module.verify() {
        return Err(codegen_err(format!("LLVM verifier errors: {errors}")));
    }
    Ok(emitter.module)
}

/// 便捷入口：生成 LLVM IR 文本（`XULO_DUMP_LLVM` 与 golden test 使用）。
pub fn emit_text(ir: &IrModule) -> Result<String, XuloError> {
    let context = Context::create();
    let module = build_module(&context, ir)?;
    Ok(module.print_to_string().to_string())
}

/// 生成目标文件（.o）。`optimize` 对应 debug/release profile。
pub fn emit_object(ir: &IrModule, obj_path: &Path, optimize: bool) -> Result<(), XuloError> {
    let context = Context::create();
    let module = build_module(&context, ir)?;
    object::write_object(&module, obj_path, optimize)
}

struct Emitter<'a, 'ctx> {
    ir: &'a IrModule,
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    runtime: HashMap<String, RuntimeSig<'ctx>>,
    externs: Vec<FunctionValue<'ctx>>,
    funcs: Vec<FunctionValue<'ctx>>,
    strings: HashMap<String, PointerValue<'ctx>>,
    next_string: usize,
    trunc_f64: FunctionValue<'ctx>,
    fptosi_sat: FunctionValue<'ctx>,
    trap_fn: FunctionValue<'ctx>,
    cur_fn: Option<FunctionValue<'ctx>>,
    cur_void: bool,
    local_slots: HashMap<LocalId, Slot<'ctx>>,
    label_blocks: HashMap<Label, BasicBlock<'ctx>>,
}

impl<'a, 'ctx> Emitter<'a, 'ctx> {
    fn new(context: &'ctx Context, ir: &'a IrModule) -> Self {
        let module = context.create_module("xulo_program");
        let builder = context.create_builder();

        let trunc_ty = context
            .f64_type()
            .fn_type(&[context.f64_type().into()], false);
        let trunc_f64 = module.add_function("llvm.trunc.f64", trunc_ty, Some(Linkage::External));
        let sat_ty = context
            .i64_type()
            .fn_type(&[context.f64_type().into()], false);
        let fptosi_sat =
            module.add_function("llvm.fptosi.sat.i64.f64", sat_ty, Some(Linkage::External));
        let trap_ty = context.void_type().fn_type(&[], false);
        let trap_fn = module.add_function("llvm.trap", trap_ty, Some(Linkage::External));

        Emitter {
            ir,
            context,
            module,
            builder,
            runtime: HashMap::new(),
            externs: Vec::new(),
            funcs: Vec::new(),
            strings: HashMap::new(),
            next_string: 0,
            trunc_f64,
            fptosi_sat,
            trap_fn,
            cur_fn: None,
            cur_void: false,
            local_slots: HashMap::new(),
            label_blocks: HashMap::new(),
        }
    }

    fn declare_functions(&mut self) -> Result<(), XuloError> {
        let mut used: HashMap<String, usize> = HashMap::new();
        for func in &self.ir.functions {
            let base = aot_symbol_name(&func.name);
            let count = {
                let entry = used.entry(base.clone()).or_insert(0);
                *entry += 1;
                *entry
            };
            let symbol = if count == 1 {
                base
            } else {
                format!("{base}_{count}")
            };

            let i64_ty = self.context.i64_type();
            let fn_ty = if func.closure_entry {
                let params: Vec<BasicMetadataTypeEnum<'ctx>> =
                    (0..9).map(|_| i64_ty.into()).collect();
                i64_ty.fn_type(&params, false)
            } else {
                let params: Vec<BasicMetadataTypeEnum<'ctx>> =
                    func.params.iter().map(|_| i64_ty.into()).collect();
                if func.return_type.is_void() {
                    self.context.void_type().fn_type(&params, false)
                } else {
                    i64_ty.fn_type(&params, false)
                }
            };
            let fv = self
                .module
                .add_function(&symbol, fn_ty, Some(Linkage::External));
            self.funcs.push(fv);
        }
        Ok(())
    }

    fn lower_functions(&mut self) -> Result<(), XuloError> {
        let entry = self.ir.entry_point.0;
        for (idx, func) in self.ir.functions.iter().enumerate() {
            self.lower_function(func, idx, idx == entry)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum NumericOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Copy)]
enum CompareOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Lte,
    Gte,
}

#[derive(Clone, Copy)]
enum StoreMode {
    Coerce,
    Fit,
}
