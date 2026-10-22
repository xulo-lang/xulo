//! IR -> Cranelift 函数体 lowering（JIT / AOT 共用一份逻辑）。
//!
//! 模块级管理（声明/定义函数、字符串常量、符号）在 `backend::CompilerModule`；
//! 本文件只关心“怎么把一条条 xulo IR 变成 Cranelift 指令”。

use std::collections::HashMap;

use cranelift::codegen::ir::condcodes::FloatCC;
use cranelift::codegen::ir::condcodes::*;
use cranelift::codegen::ir::entities::Block;
use cranelift::codegen::ir::types::{F64, I64, Type};
use cranelift::codegen::ir::{InstBuilder, Signature, Value};
use cranelift::codegen::isa::CallConv;
use cranelift::codegen::verify_function;
use cranelift::frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{FuncId, Linkage};
use cranelift_object::ObjectProduct;

use xulo_core::error::{ErrorKind, XuloError};
use xulo_core::ir::*;

use crate::backend::{BackendMode, CompilerModule, RUNTIME_FNS};

fn block_has_terminator(func: &cranelift::codegen::ir::Function, block: Block) -> bool {
    if let Some(inst) = func.layout.last_inst(block) {
        func.dfg.insts[inst].opcode().is_terminator()
    } else {
        false
    }
}

pub struct CodeGen {
    backend: CompilerModule,
    builder_ctx: FunctionBuilderContext,
    runtime_func_ids: HashMap<String, FuncId>,
    /// 运行时函数签名（参数/返回的 Cranelift 类型），RuntimeCall 处做实参对齐
    runtime_sigs: HashMap<String, (Vec<Type>, Vec<Type>)>,
    /// IR 字符串表（对应 ir_module.strings）；入口处注册给运行时做自动类型检测
    strings: Vec<String>,
}

impl Default for CodeGen {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGen {
    /// JIT 后端（进程内执行）
    pub fn new() -> Self {
        Self::with_mode(BackendMode::Jit).expect("failed to init JIT backend")
    }

    /// 指定后端模式
    pub fn with_mode(mode: BackendMode) -> Result<Self, XuloError> {
        Ok(CodeGen {
            backend: CompilerModule::new(mode)?,
            builder_ctx: FunctionBuilderContext::new(),
            runtime_func_ids: HashMap::new(),
            runtime_sigs: HashMap::new(),
            strings: Vec::new(),
        })
    }

    /// JIT 模式：编译整个模块并返回入口函数指针
    pub fn compile(&mut self, ir_module: &IrModule) -> Result<*const u8, XuloError> {
        if self.backend.mode() != BackendMode::Jit {
            return Err(XuloError::new(
                ErrorKind::Codegen,
                "compile 只支持 JIT 后端；AOT 请用 compile_aot",
            ));
        }
        let entry = self.compile_all(ir_module)?;
        self.backend.finalize()?;
        Ok(self.backend.finalized_entry(entry))
    }

    /// AOT 模式：编译整个模块，产出目标文件数据
    pub fn compile_aot(mut self, ir_module: &IrModule) -> Result<ObjectProduct, XuloError> {
        if self.backend.mode() != BackendMode::Aot {
            return Err(XuloError::new(
                ErrorKind::Codegen,
                "compile_aot 只支持 AOT 后端；JIT 请用 compile",
            ));
        }
        self.compile_all(ir_module)?;
        Ok(self.backend.finish())
    }

    /// 两后端共用的编译流程：字符串 -> 声明 -> 函数体；返回入口函数 id
    fn compile_all(&mut self, ir_module: &IrModule) -> Result<FuncId, XuloError> {
        // 字符串常量表：两后端各自 intern（JIT 进程内指针 / AOT .rodata）
        self.strings.clear();
        for s in &ir_module.strings {
            self.backend.intern_string(s)?;
            self.strings.push(s.clone());
        }

        // 1. 声明所有用户函数
        let mut func_ids = Vec::new();
        for func in &ir_module.functions {
            let sig = self.create_signature(func);
            let func_id = self
                .backend
                .declare_function(&func.name, Linkage::Export, &sig)?;
            func_ids.push(func_id);
        }

        // 2. 声明运行时函数
        self.declare_runtime_functions()?;

        // 3. 编译所有函数体
        for (i, func) in ir_module.functions.iter().enumerate() {
            self.compile_function(func, func_ids[i], &func_ids)?;
        }

        Ok(func_ids[ir_module.entry_point.0])
    }

    fn declare_runtime_functions(&mut self) -> Result<(), XuloError> {
        for desc in RUNTIME_FNS {
            let mut sig = Signature::new(CallConv::SystemV);
            for pt in desc.params {
                sig.params.push(cranelift::codegen::ir::AbiParam::new(*pt));
            }
            for rt in desc.returns {
                sig.returns.push(cranelift::codegen::ir::AbiParam::new(*rt));
            }
            let id = self
                .backend
                .declare_function(desc.name, Linkage::Import, &sig)?;
            self.runtime_func_ids.insert(desc.name.to_string(), id);
            self.runtime_sigs.insert(
                desc.name.to_string(),
                (desc.params.to_vec(), desc.returns.to_vec()),
            );
        }
        Ok(())
    }

    /// 物化目标函数的地址（I64 指针）：以 namespace-0 用户符号创建
    /// `GlobalValueData::Symbol`，JIT 在 finalize 时解析为编译后入口，
    /// AOT 经系统链接器解析
    fn function_addr(func_id: FuncId, builder: &mut FunctionBuilder) -> Value {
        let user_name_ref =
            builder
                .func
                .declare_imported_user_function(cranelift::codegen::ir::UserExternalName {
                    namespace: 0,
                    index: func_id.as_u32(),
                });
        let gv = builder.create_global_value(cranelift::codegen::ir::GlobalValueData::Symbol {
            name: cranelift::codegen::ir::ExternalName::user(user_name_ref),
            offset: cranelift::codegen::ir::immediates::Imm64::new(0),
            colocated: true,
            tls: false,
        });
        builder.ins().global_value(I64, gv)
    }

    fn create_signature(&self, func: &IrFunction) -> Signature {
        let mut sig = Signature::new(CallConv::SystemV);
        if func.closure_entry {
            // 闭包入口固定 `(env, a0..a7) -> i64`：CallIndirect 总是传满 9 个
            // I64 实参（缺省补 0），无返回值也返回 0
            for _ in 0..9 {
                sig.params.push(cranelift::codegen::ir::AbiParam::new(I64));
            }
            sig.returns.push(cranelift::codegen::ir::AbiParam::new(I64));
            return sig;
        }
        for _ in &func.params {
            sig.params.push(cranelift::codegen::ir::AbiParam::new(I64));
        }
        if !func.return_type.is_void() {
            sig.returns.push(cranelift::codegen::ir::AbiParam::new(I64));
        }
        sig
    }

    fn compile_function(
        &mut self,
        ir_func: &IrFunction,
        func_id: FuncId,
        all_func_ids: &[FuncId],
    ) -> Result<(), XuloError> {
        let sig = self.create_signature(ir_func);
        let mut ctx = self.backend.make_context();
        ctx.func = cranelift::codegen::ir::Function::with_name_signature(
            cranelift::codegen::ir::UserFuncName::testcase(ir_func.name.clone()),
            sig,
        );

        let mut vars: HashMap<LocalId, Variable> = HashMap::new();
        let mut var_types: Vec<cranelift::codegen::ir::Type> = Vec::new();
        let mut block_map: HashMap<Label, cranelift::codegen::ir::Block> = HashMap::new();

        for (i, (local_id, ty)) in ir_func.locals.iter().enumerate() {
            let var = Variable::from_u32(i as u32);
            let cranelift_type = match ty {
                IrType::I64 => I64,
                IrType::F64 => cranelift::codegen::ir::types::F64,
                _ => I64,
            };
            var_types.push(cranelift_type);
            vars.insert(*local_id, var);
        }

        for (i, (local_id, _)) in ir_func.params.iter().enumerate() {
            let var = Variable::from_u32((ir_func.locals.len() + i) as u32);
            var_types.push(I64);
            vars.insert(*local_id, var);
        }

        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut self.builder_ctx);

        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);

        // 注册所有字符串指针到运行时（用于自动类型检测）
        if let Some(&reg_id) = self.runtime_func_ids.get("xulo_register_string") {
            let fref = self.backend.declare_func_in_func(reg_id, builder.func);
            for s in &self.strings {
                let ptr_val = self.backend.string_value(s, &mut builder);
                builder.ins().call(fref, &[ptr_val]);
            }
        }

        for label in &ir_func.labels {
            let block = builder.create_block();
            block_map.insert(*label, block);
        }

        // 声明所有变量的类型
        for (idx, (local_id, _ty)) in ir_func.locals.iter().enumerate() {
            if let Some(&var) = vars.get(local_id)
                && idx < var_types.len()
            {
                builder.declare_var(var, var_types[idx]);
            }
        }

        // 声明参数变量
        for (local_id, _) in ir_func.params.iter() {
            if let Some(&var) = vars.get(local_id) {
                builder.declare_var(var, I64);
            }
        }

        for (i, (local_id, _)) in ir_func.params.iter().enumerate() {
            if let Some(&var) = vars.get(local_id) {
                let param_val = builder.block_params(entry_block)[i];
                builder.def_var(var, param_val);
            }
        }

        for instr in &ir_func.instructions {
            match instr {
                Instruction::Label(label) => {
                    if let Some(&block) = block_map.get(label) {
                        // If previous block is now empty, add dummy instruction
                        if let Some(cur) = builder.current_block()
                            && builder.func.layout.last_inst(cur).is_none()
                            && cur != entry_block
                        {
                            builder.ins().iconst(I64, 0);
                        }
                        builder.switch_to_block(block);
                    }
                }
                Instruction::Const { dst, value } => {
                    let val = match value {
                        IrValue::Bool(b) => builder.ins().iconst(I64, if *b { 1 } else { 0 }),
                        IrValue::I64(n) => builder.ins().iconst(I64, *n),
                        IrValue::F64(n) => {
                            let bits = n.to_bits() as i64;
                            let v = builder.ins().f64const(*n);
                            // 注册浮点数位模式，用于自动检测
                            if let Some(&reg_id) = self.runtime_func_ids.get("xulo_register_float")
                            {
                                let fref = self.backend.declare_func_in_func(reg_id, builder.func);
                                let bits_val = builder.ins().iconst(I64, bits);
                                builder.ins().call(fref, &[bits_val]);
                            }
                            v
                        }
                        IrValue::Null => builder.ins().iconst(I64, 0),
                        IrValue::String(idx) => match self.strings.get(*idx) {
                            Some(s) => self.backend.string_value(s, &mut builder),
                            None => builder.ins().iconst(I64, 0),
                        },
                    };
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Add { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = numeric_binop(&mut builder, l, r, NumericOp::Add);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Sub { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = numeric_binop(&mut builder, l, r, NumericOp::Sub);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Mul { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = numeric_binop(&mut builder, l, r, NumericOp::Mul);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Div { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = numeric_binop(&mut builder, l, r, NumericOp::Div);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Mod { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().srem(l, r);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Pow { dst, left, right } => {
                    // Cranelift 无整数幂指令；与解释器一致经 powf 计算，
                    // 整数槽截断返回，F64 槽按位模式承载
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let (lf, rf) = promote_pair(&mut builder, l, r);
                    if let Some(&func_id) = self.runtime_func_ids.get("xulo_math_pow") {
                        let fref = self.backend.declare_func_in_func(func_id, builder.func);
                        let call = builder.ins().call(fref, &[lf, rf]);
                        let result_f = builder.inst_results(call)[0];
                        if let Some(&var) = vars.get(dst) {
                            let dst_ty = var_types
                                .get(var.as_u32() as usize)
                                .copied()
                                .unwrap_or(I64);
                            let val = if dst_ty == F64 {
                                result_f
                            } else {
                                builder.ins().fcvt_to_sint(I64, result_f)
                            };
                            builder.def_var(var, val);
                        }
                    }
                }
                Instruction::Neg { dst, operand } => {
                    let o = resolve_operand(&mut builder, operand, &vars);
                    let val = if builder.func.dfg.value_type(o) == F64 {
                        builder.ins().fneg(o)
                    } else {
                        builder.ins().ineg(o)
                    };
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::BitAnd { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().band(l, r);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::BitOr { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().bor(l, r);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Xor { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().bxor(l, r);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Shl { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().ishl(l, r);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Shr { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().sshr(l, r);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::BitNot { dst, operand } => {
                    let o = resolve_operand(&mut builder, operand, &vars);
                    let val = builder.ins().bnot(o);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Eq { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let cmp = if builder.func.dfg.value_type(l) == F64
                        || builder.func.dfg.value_type(r) == F64
                    {
                        let (l, r) = promote_pair(&mut builder, l, r);
                        builder.ins().fcmp(FloatCC::Equal, l, r)
                    } else {
                        builder.ins().icmp(IntCC::Equal, l, r)
                    };
                    let val = builder.ins().uextend(I64, cmp);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Neq { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let cmp = if builder.func.dfg.value_type(l) == F64
                        || builder.func.dfg.value_type(r) == F64
                    {
                        let (l, r) = promote_pair(&mut builder, l, r);
                        builder.ins().fcmp(FloatCC::NotEqual, l, r)
                    } else {
                        builder.ins().icmp(IntCC::NotEqual, l, r)
                    };
                    let val = builder.ins().uextend(I64, cmp);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Lt { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let cmp = if builder.func.dfg.value_type(l) == F64
                        || builder.func.dfg.value_type(r) == F64
                    {
                        let (l, r) = promote_pair(&mut builder, l, r);
                        builder.ins().fcmp(FloatCC::LessThan, l, r)
                    } else {
                        builder.ins().icmp(IntCC::SignedLessThan, l, r)
                    };
                    let val = builder.ins().uextend(I64, cmp);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Gt { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let cmp = if builder.func.dfg.value_type(l) == F64
                        || builder.func.dfg.value_type(r) == F64
                    {
                        let (l, r) = promote_pair(&mut builder, l, r);
                        builder.ins().fcmp(FloatCC::GreaterThan, l, r)
                    } else {
                        builder.ins().icmp(IntCC::SignedGreaterThan, l, r)
                    };
                    let val = builder.ins().uextend(I64, cmp);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Lte { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let cmp = if builder.func.dfg.value_type(l) == F64
                        || builder.func.dfg.value_type(r) == F64
                    {
                        let (l, r) = promote_pair(&mut builder, l, r);
                        builder.ins().fcmp(FloatCC::LessThanOrEqual, l, r)
                    } else {
                        builder.ins().icmp(IntCC::SignedLessThanOrEqual, l, r)
                    };
                    let val = builder.ins().uextend(I64, cmp);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Gte { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let cmp = if builder.func.dfg.value_type(l) == F64
                        || builder.func.dfg.value_type(r) == F64
                    {
                        let (l, r) = promote_pair(&mut builder, l, r);
                        builder.ins().fcmp(FloatCC::GreaterThanOrEqual, l, r)
                    } else {
                        builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, l, r)
                    };
                    let val = builder.ins().uextend(I64, cmp);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::And { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().band(l, r);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Or { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().bor(l, r);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Not { dst, operand } => {
                    let o = resolve_operand(&mut builder, operand, &vars);
                    // 逻辑取反：0 -> 1，非 0 -> 0（bnot 会把 true(1) 变成 -2）
                    let is_zero = builder.ins().icmp_imm(IntCC::Equal, o, 0);
                    let one = builder.ins().iconst(I64, 1);
                    let zero = builder.ins().iconst(I64, 0);
                    let val = builder.ins().select(is_zero, one, zero);
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Branch {
                    condition,
                    then_label,
                    else_label,
                } => {
                    let cond = resolve_operand(&mut builder, condition, &vars);
                    let cond_bool = builder.ins().icmp_imm(IntCC::NotEqual, cond, 0);
                    let then_block = block_map[then_label];
                    let else_block = block_map[else_label];
                    builder
                        .ins()
                        .brif(cond_bool, then_block, &[], else_block, &[]);
                }
                Instruction::Jump(label) => {
                    let block = block_map[label];
                    builder.ins().jump(block, &[]);
                }
                Instruction::Return(value) => {
                    if let Some(v) = value {
                        let mut ret_val = resolve_operand(&mut builder, v, &vars);
                        if let Some(sig_ret) = builder.func.signature.returns.first().copied() {
                            ret_val = coerce_to(&mut builder, ret_val, sig_ret.value_type);
                        }
                        builder.ins().return_(&[ret_val]);
                    } else if builder.func.signature.returns.is_empty() {
                        builder.ins().return_(&[]);
                    } else {
                        // 无值 return 但签名有返回值（闭包入口）：补 0
                        let ret_ty = builder.func.signature.returns[0].value_type;
                        let zero = builder.ins().iconst(ret_ty, 0);
                        builder.ins().return_(&[zero]);
                    }
                }
                Instruction::SetLocal { dst, src } => {
                    let val = resolve_operand(&mut builder, src, &vars);
                    if let Some(&var) = vars.get(dst) {
                        let dst_ty = var_types.get(var.as_u32() as usize).copied().unwrap_or(I64);
                        let val = coerce_to(&mut builder, val, dst_ty);
                        builder.def_var(var, val);
                    }
                }
                Instruction::GetLocal { dst, src } => {
                    if let Some(&src_var) = vars.get(src) {
                        let val = builder.use_var(src_var);
                        if let Some(&dst_var) = vars.get(dst) {
                            builder.def_var(dst_var, val);
                        }
                    }
                }
                Instruction::RuntimeCall { dst, func, args } => {
                    let arg_vals: Vec<_> = args
                        .iter()
                        .map(|a| resolve_operand(&mut builder, a, &vars))
                        .collect();

                    let fn_name = match func {
                        RuntimeFn::Print => "xulo_print",
                        RuntimeFn::PrintInt => "xulo_print_int",
                        RuntimeFn::PrintFloat => "xulo_print_float",
                        RuntimeFn::PrintValue => "xulo_print_value",
                        RuntimeFn::Panic => "xulo_panic",
                        RuntimeFn::StringConcat => "xulo_string_concat",
                        RuntimeFn::Stringify => "xulo_stringify",
                        RuntimeFn::AllocObject => "xulo_alloc_object",
                        RuntimeFn::AllocArray => "xulo_alloc_array",
                        RuntimeFn::ArrayPush => "xulo_array_push",
                        RuntimeFn::ArrayLen => "xulo_array_len",
                        RuntimeFn::ArrayGet => "xulo_array_get",
                        RuntimeFn::ArrayGetTag => "xulo_array_get_tag",
                        RuntimeFn::ArraySet => "xulo_array_set",
                        RuntimeFn::ArrayConcat => "xulo_array_concat",
                        RuntimeFn::ObjectGet => "xulo_object_get",
                        RuntimeFn::ObjectGetTag => "xulo_object_get_tag",
                        RuntimeFn::ObjectSet => "xulo_object_set",
                        RuntimeFn::AllocEnum => "xulo_alloc_enum",
                        RuntimeFn::EnumSet => "xulo_enum_set",
                        RuntimeFn::EnumVariant => "xulo_enum_variant",
                        RuntimeFn::EnumGet => "xulo_enum_get",
                        RuntimeFn::EnumGetTag => "xulo_enum_get_tag",
                        RuntimeFn::AsF64 => "xulo_as_f64",
                        RuntimeFn::RegisterFloat => "xulo_register_float",
                        RuntimeFn::AllocCell => "xulo_alloc_cell",
                        RuntimeFn::CellGet => "xulo_cell_get",
                        RuntimeFn::CellGetF64 => "xulo_cell_get_f64",
                        RuntimeFn::CellSet => "xulo_cell_set",
                        RuntimeFn::AllocEnv => "xulo_alloc_env",
                        RuntimeFn::EnvSet => "xulo_env_set",
                        RuntimeFn::EnvGet => "xulo_env_get",
                        RuntimeFn::AllocClosure => "xulo_alloc_closure",
                        RuntimeFn::ClosureFunc => "xulo_closure_func",
                        RuntimeFn::ClosureEnv => "xulo_closure_env",
                        RuntimeFn::CheckCallable => "xulo_check_callable",
                        RuntimeFn::RegexCompile => "xulo_regex_compile",
                        RuntimeFn::RegexTest => "xulo_regex_test",
                        RuntimeFn::RegexFind => "xulo_regex_find",
                        RuntimeFn::RegexFindAll => "xulo_regex_find_all",
                        RuntimeFn::RegexMatchGroup => "xulo_regex_match_group",
                        RuntimeFn::RegexReplace => "xulo_regex_replace",
                        RuntimeFn::RegexReplaceWith => "xulo_regex_replace_with",
                        RuntimeFn::StringToUppercase => "xulo_string_to_uppercase",
                        RuntimeFn::LiteralMatch => "xulo_literal_match",
                        RuntimeFn::RangeList => "xulo_range_list",
                        RuntimeFn::Intrinsic(name) => name,
                    };

                    if let Some(&func_id) = self.runtime_func_ids.get(fn_name) {
                        let (decl_params, decl_returns) =
                            self.runtime_sigs.get(fn_name).cloned().unwrap_or_default();

                        // 实参对齐声明的参数类型：int 上下文按数值转进 f64 形参，
                        // f64 值进 i64 形参（打印/入槽）按位模式
                        let final_args: Vec<_> = arg_vals
                            .iter()
                            .enumerate()
                            .map(|(i, v)| match decl_params.get(i) {
                                Some(&declared) => coerce_to(&mut builder, *v, declared),
                                None => *v,
                            })
                            .collect();

                        let fref = self.backend.declare_func_in_func(func_id, builder.func);
                        let call = builder.ins().call(fref, &final_args);
                        let call_result = builder.inst_results(call).first().copied();
                        if let Some(d) = dst
                            && let Some(&var) = vars.get(d)
                            && !decl_returns.is_empty()
                            && let Some(result) = call_result
                        {
                            // 返回值对齐 dst 槽类型（f64 结果按位模式进 I64 槽）
                            let dst_ty =
                                var_types.get(var.as_u32() as usize).copied().unwrap_or(I64);
                            let coerced = coerce_to(&mut builder, result, dst_ty);
                            builder.def_var(var, coerced);
                        }
                    }
                }
                Instruction::Call { dst, func, args } => {
                    let arg_vals: Vec<_> = args
                        .iter()
                        .map(|a| {
                            let v = resolve_operand(&mut builder, a, &vars);
                            coerce_to(&mut builder, v, I64)
                        })
                        .collect();

                    if let Some(call_id) = all_func_ids.get(func.0) {
                        let call_id = *call_id;
                        let func_ref = self.backend.declare_func_in_func(call_id, builder.func);
                        let call = builder.ins().call(func_ref, &arg_vals);

                        if let Some(d) = dst
                            && let Some(&var) = vars.get(d)
                            && let Some(result) = builder.inst_results(call).first()
                        {
                            builder.def_var(var, *result);
                        }
                    }
                }
                Instruction::NewObject { dst, fields } => {
                    let size_val = builder.ins().iconst(I64, 0);
                    if let Some(&func_id) = self.runtime_func_ids.get("xulo_alloc_object") {
                        let fref = self.backend.declare_func_in_func(func_id, builder.func);
                        let call = builder.ins().call(fref, &[size_val]);
                        if let Some(&var) = vars.get(dst)
                            && let Some(result) = builder.inst_results(call).first()
                        {
                            builder.def_var(var, *result);
                            for (field_name, field_val, tag) in fields.iter() {
                                let obj_val = builder.use_var(var);
                                let raw_val = resolve_operand(&mut builder, field_val, &vars);
                                let val = coerce_to(&mut builder, raw_val, I64);
                                let tag_val = builder.ins().iconst(I64, *tag);
                                let field_ptr = self.backend.string_value(field_name, &mut builder);
                                if let Some(&set_id) = self.runtime_func_ids.get("xulo_object_set")
                                {
                                    let fref2 =
                                        self.backend.declare_func_in_func(set_id, builder.func);
                                    builder
                                        .ins()
                                        .call(fref2, &[obj_val, field_ptr, val, tag_val]);
                                }
                            }
                        }
                    }
                }
                Instruction::NewArray { dst, elements } => {
                    let size_val = builder.ins().iconst(I64, 0);
                    if let Some(&func_id) = self.runtime_func_ids.get("xulo_alloc_array") {
                        let fref = self.backend.declare_func_in_func(func_id, builder.func);
                        let call = builder.ins().call(fref, &[size_val]);
                        if let Some(&var) = vars.get(dst)
                            && let Some(result) = builder.inst_results(call).first()
                        {
                            builder.def_var(var, *result);
                            for (elem, tag) in elements.iter() {
                                let arr_val = builder.use_var(var);
                                let raw_elem = resolve_operand(&mut builder, elem, &vars);
                                let elem_val = coerce_to(&mut builder, raw_elem, I64);
                                let tag_val = builder.ins().iconst(I64, *tag);
                                if let Some(&push_id) = self.runtime_func_ids.get("xulo_array_push")
                                {
                                    let fref2 =
                                        self.backend.declare_func_in_func(push_id, builder.func);
                                    builder.ins().call(fref2, &[arr_val, elem_val, tag_val]);
                                }
                            }
                        }
                    }
                }
                Instruction::GetField { dst, object, field } => {
                    let obj_val = resolve_operand(&mut builder, object, &vars);
                    let field_val = self.backend.string_value(field, &mut builder);
                    if let Some(&func_id) = self.runtime_func_ids.get("xulo_object_get") {
                        let fref = self.backend.declare_func_in_func(func_id, builder.func);
                        let call = builder.ins().call(fref, &[obj_val, field_val]);
                        if let Some(&var) = vars.get(dst)
                            && let Some(result) = builder.inst_results(call).first()
                        {
                            builder.def_var(var, *result);
                        }
                    }
                }
                Instruction::SetField {
                    object,
                    field,
                    value,
                    tag,
                } => {
                    let obj_val = resolve_operand(&mut builder, object, &vars);
                    let raw_val = resolve_operand(&mut builder, value, &vars);
                    let val = coerce_to(&mut builder, raw_val, I64);
                    let tag_val = builder.ins().iconst(I64, *tag);
                    let field_val = self.backend.string_value(field, &mut builder);
                    if let Some(&func_id) = self.runtime_func_ids.get("xulo_object_set") {
                        let fref = self.backend.declare_func_in_func(func_id, builder.func);
                        builder
                            .ins()
                            .call(fref, &[obj_val, field_val, val, tag_val]);
                    }
                }
                Instruction::GetIndex { dst, array, index } => {
                    let arr_val = resolve_operand(&mut builder, array, &vars);
                    let idx_val = resolve_operand(&mut builder, index, &vars);
                    if let Some(&func_id) = self.runtime_func_ids.get("xulo_array_get") {
                        let fref = self.backend.declare_func_in_func(func_id, builder.func);
                        let call = builder.ins().call(fref, &[arr_val, idx_val]);
                        if let Some(&var) = vars.get(dst)
                            && let Some(result) = builder.inst_results(call).first()
                        {
                            builder.def_var(var, *result);
                        }
                    }
                }
                Instruction::SetIndex {
                    array,
                    index,
                    value,
                    tag,
                } => {
                    let arr_val = resolve_operand(&mut builder, array, &vars);
                    let idx_val = resolve_operand(&mut builder, index, &vars);
                    let raw_val = resolve_operand(&mut builder, value, &vars);
                    let val = coerce_to(&mut builder, raw_val, I64);
                    let tag_val = builder.ins().iconst(I64, *tag);
                    if let Some(&func_id) = self.runtime_func_ids.get("xulo_array_set") {
                        let fref = self.backend.declare_func_in_func(func_id, builder.func);
                        builder.ins().call(fref, &[arr_val, idx_val, val, tag_val]);
                    }
                }
                Instruction::MakeClosure { dst, func, env } => {
                    let env_val = resolve_operand(&mut builder, env, &vars);
                    if let Some(&alloc_id) = self.runtime_func_ids.get("xulo_alloc_closure") {
                        let func_addr = Self::function_addr(all_func_ids[func.0], &mut builder);
                        let fref = self.backend.declare_func_in_func(alloc_id, builder.func);
                        let call = builder.ins().call(fref, &[func_addr, env_val]);
                        if let Some(&var) = vars.get(dst)
                            && let Some(result) = builder.inst_results(call).first()
                        {
                            builder.def_var(var, *result);
                        }
                    }
                }
                Instruction::CallIndirect { dst, callee, args } => {
                    let callee_val = resolve_operand(&mut builder, callee, &vars);
                    let mut cl_args: Vec<Value> = Vec::with_capacity(9);
                    let mut call_target = builder.ins().iconst(I64, 0);
                    if let Some(&check_id) = self.runtime_func_ids.get("xulo_check_callable")
                        && let Some(&func_id) = self.runtime_func_ids.get("xulo_closure_func")
                        && let Some(&env_id) = self.runtime_func_ids.get("xulo_closure_env")
                    {
                        // 校验 + 拆出函数地址与环境指针
                        let check_ref = self.backend.declare_func_in_func(check_id, builder.func);
                        let checked = {
                            let call = builder.ins().call(check_ref, &[callee_val]);
                            builder.inst_results(call)[0]
                        };
                        let func_ref = self.backend.declare_func_in_func(func_id, builder.func);
                        call_target = {
                            let call = builder.ins().call(func_ref, &[checked]);
                            builder.inst_results(call)[0]
                        };
                        let env_ref = self.backend.declare_func_in_func(env_id, builder.func);
                        let env_ptr = {
                            let call = builder.ins().call(env_ref, &[checked]);
                            builder.inst_results(call)[0]
                        };
                        cl_args.push(env_ptr);
                        for a in args.iter().take(8) {
                            let v = resolve_operand(&mut builder, a, &vars);
                            cl_args.push(coerce_to(&mut builder, v, I64));
                        }
                    } else {
                        cl_args.push(builder.ins().iconst(I64, 0));
                    }
                    // 不足 8 个实参补 0（闭包入口签名固定 9 个 I64）
                    while cl_args.len() < 9 {
                        cl_args.push(builder.ins().iconst(I64, 0));
                    }
                    let mut sig = Signature::new(CallConv::SystemV);
                    for _ in 0..9 {
                        sig.params.push(cranelift::codegen::ir::AbiParam::new(I64));
                    }
                    sig.returns.push(cranelift::codegen::ir::AbiParam::new(I64));
                    let sig_ref = builder.import_signature(sig);
                    let call = builder.ins().call_indirect(sig_ref, call_target, &cl_args);
                    if let Some(d) = dst
                        && let Some(&var) = vars.get(d)
                        && let Some(result) = builder.inst_results(call).first().copied()
                    {
                        let dst_ty = var_types.get(var.as_u32() as usize).copied().unwrap_or(I64);
                        let coerced = coerce_to(&mut builder, result, dst_ty);
                        builder.def_var(var, coerced);
                    }
                }
                Instruction::Nop => {}
            }
        }

        // Ensure ALL blocks have terminators
        // First, ensure current block (last label block) has a terminator
        if let Some(current_block) = builder.current_block()
            && !block_has_terminator(builder.func, current_block)
        {
            if builder.func.signature.returns.is_empty() {
                builder.ins().return_(&[]);
            } else {
                let ret_ty = builder.func.signature.returns[0].value_type;
                let zero = builder.ins().iconst(ret_ty, 0);
                builder.ins().return_(&[zero]);
            }
        }
        // Also ensure entry_block has a terminator (it may have been switched away from)
        if !block_has_terminator(builder.func, entry_block) {
            if let Some(&first_label) = ir_func.labels.first() {
                if let Some(&first_block) = block_map.get(&first_label) {
                    builder.switch_to_block(entry_block);
                    builder.ins().jump(first_block, &[]);
                }
            } else {
                builder.switch_to_block(entry_block);
                if builder.func.signature.returns.is_empty() {
                    builder.ins().return_(&[]);
                } else {
                    let ret_ty = builder.func.signature.returns[0].value_type;
                    let zero = builder.ins().iconst(ret_ty, 0);
                    builder.ins().return_(&[zero]);
                }
            }
        }

        builder.seal_all_blocks();
        builder.finalize();

        // Run verifier to get detailed error messages
        if let Err(errors) = verify_function(&ctx.func, self.backend.isa()) {
            return Err(XuloError::new(
                ErrorKind::Codegen,
                format!("verifier errors for {}: {}", ir_func.name, errors),
            ));
        }

        self.backend.define_function(func_id, &mut ctx)?;

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

/// 数值算术：两侧都是整数走整数指令；任一侧是 f64（或动态转换出来的
/// f64 操作数）则把整数侧按数值提升后走浮点指令。
fn numeric_binop(builder: &mut FunctionBuilder, l: Value, r: Value, op: NumericOp) -> Value {
    let lt = builder.func.dfg.value_type(l);
    let rt = builder.func.dfg.value_type(r);
    if lt == F64 || rt == F64 {
        let (l, r) = promote_pair(builder, l, r);
        match op {
            NumericOp::Add => builder.ins().fadd(l, r),
            NumericOp::Sub => builder.ins().fsub(l, r),
            NumericOp::Mul => builder.ins().fmul(l, r),
            NumericOp::Div => builder.ins().fdiv(l, r),
        }
    } else {
        match op {
            NumericOp::Add => builder.ins().iadd(l, r),
            NumericOp::Sub => builder.ins().isub(l, r),
            NumericOp::Mul => builder.ins().imul(l, r),
            NumericOp::Div => builder.ins().sdiv(l, r),
        }
    }
}

/// 把混合的 (I64, F64) 操作数对齐到双 f64（整数按数值转换）。
fn promote_pair(builder: &mut FunctionBuilder, l: Value, r: Value) -> (Value, Value) {
    let lt = builder.func.dfg.value_type(l);
    let rt = builder.func.dfg.value_type(r);
    let l = if lt == F64 {
        l
    } else {
        builder.ins().fcvt_from_sint(F64, l)
    };
    let r = if rt == F64 {
        r
    } else {
        builder.ins().fcvt_from_sint(F64, r)
    };
    (l, r)
}

/// 运行时调用的实参/返回值在 I64 与 F64 槽之间对齐：
/// I64 值进 f64 形参按数值转换（int 上下文进入 float 数学）；
/// f64 值进 i64 形参或 I64 槽按位模式（打印配合 tag=2 还原小数）。
fn coerce_to(builder: &mut FunctionBuilder, val: Value, declared: Type) -> Value {
    let actual = builder.func.dfg.value_type(val);
    if actual == declared {
        return val;
    }
    if declared == F64 && actual == I64 {
        builder.ins().fcvt_from_sint(F64, val)
    } else if declared == I64 && actual == F64 {
        builder
            .ins()
            .bitcast(I64, cranelift::codegen::ir::MemFlags::new(), val)
    } else {
        val
    }
}

fn resolve_operand(
    builder: &mut FunctionBuilder,
    operand: &Operand,
    vars: &HashMap<LocalId, Variable>,
) -> cranelift::codegen::ir::Value {
    match operand {
        Operand::Const(IrValue::Bool(b)) => builder.ins().iconst(I64, if *b { 1 } else { 0 }),
        Operand::Const(IrValue::I64(n)) => builder.ins().iconst(I64, *n),
        Operand::Const(IrValue::F64(n)) => builder.ins().f64const(*n),
        Operand::Const(IrValue::Null) => builder.ins().iconst(I64, 0),
        Operand::Const(IrValue::String(_idx)) => {
            // 字符串常量应该通过 Instruction::Const 处理，这里作为 fallback
            builder.ins().iconst(I64, 0)
        }
        Operand::Local(local_id) => {
            if let Some(&var) = vars.get(local_id) {
                builder.use_var(var)
            } else {
                builder.ins().iconst(I64, 0)
            }
        }
    }
}
