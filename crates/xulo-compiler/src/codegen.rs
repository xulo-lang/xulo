use std::collections::HashMap;

use cranelift::codegen::ir::types::I64;
use cranelift::codegen::ir::{Signature, InstBuilder};
use cranelift::codegen::ir::condcodes::*;
use cranelift::frontend::{FunctionBuilder, Variable, FunctionBuilderContext};
use cranelift::codegen::isa::CallConv;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Module, FuncId, Linkage};

use xulo_core::ir::*;
use xulo_core::error::{XuloError, ErrorKind};

pub struct CodeGen {
    module: JITModule,
    builder_ctx: FunctionBuilderContext,
    runtime_func_ids: HashMap<String, FuncId>,
    string_ptrs: Vec<*const u8>,
    string_field_ptrs: HashMap<String, *const u8>,
}

/// 引用 xulo-runtime，确保链接器包含运行时库
fn _ensure_runtime_linked() {
    let _ = xulo_runtime::runtime::xulo_print as *const u8;
    let _ = xulo_runtime::runtime::xulo_print_value as *const u8;
    let _ = xulo_runtime::runtime::xulo_string_concat as *const u8;
    let _ = xulo_runtime::runtime::xulo_to_string as *const u8;
    let _ = xulo_runtime::runtime::xulo_alloc_object as *const u8;
    let _ = xulo_runtime::runtime::xulo_alloc_array as *const u8;
    let _ = xulo_runtime::runtime::xulo_array_push as *const u8;
    let _ = xulo_runtime::runtime::xulo_array_len as *const u8;
    let _ = xulo_runtime::runtime::xulo_array_get as *const u8;
    let _ = xulo_runtime::runtime::xulo_array_set as *const u8;
    let _ = xulo_runtime::runtime::xulo_object_get as *const u8;
    let _ = xulo_runtime::runtime::xulo_object_set as *const u8;
}

impl CodeGen {
    pub fn new() -> Self {
        let mut builder = JITBuilder::new(cranelift_module::default_libcall_names())
            .expect("failed to create JITBuilder");

        builder.symbol("xulo_print", xulo_runtime::runtime::xulo_print as *const u8);
        builder.symbol("xulo_print_int", xulo_runtime::runtime::xulo_print_int as *const u8);
        builder.symbol("xulo_print_float", xulo_runtime::runtime::xulo_print_float as *const u8);
        builder.symbol("xulo_print_value", xulo_runtime::runtime::xulo_print_value as *const u8);
        builder.symbol("xulo_panic", xulo_runtime::runtime::xulo_panic as *const u8);
        builder.symbol("xulo_string_concat", xulo_runtime::runtime::xulo_string_concat as *const u8);
        builder.symbol("xulo_to_string", xulo_runtime::runtime::xulo_to_string as *const u8);
        builder.symbol("xulo_alloc_object", xulo_runtime::runtime::xulo_alloc_object as *const u8);
        builder.symbol("xulo_alloc_array", xulo_runtime::runtime::xulo_alloc_array as *const u8);
        builder.symbol("xulo_array_push", xulo_runtime::runtime::xulo_array_push as *const u8);
        builder.symbol("xulo_array_len", xulo_runtime::runtime::xulo_array_len as *const u8);
        builder.symbol("xulo_array_get", xulo_runtime::runtime::xulo_array_get as *const u8);
        builder.symbol("xulo_array_set", xulo_runtime::runtime::xulo_array_set as *const u8);
        builder.symbol("xulo_array_concat", xulo_runtime::runtime::xulo_array_concat as *const u8);
        builder.symbol("xulo_object_get", xulo_runtime::runtime::xulo_object_get as *const u8);
        builder.symbol("xulo_object_set", xulo_runtime::runtime::xulo_object_set as *const u8);
        builder.symbol("xulo_register_float", xulo_runtime::runtime::xulo_register_float as *const u8);
        builder.symbol("xulo_register_string", xulo_runtime::runtime::xulo_register_string as *const u8);

        let module = JITModule::new(builder);

        CodeGen {
            module,
            builder_ctx: FunctionBuilderContext::new(),
            runtime_func_ids: HashMap::new(),
            string_ptrs: Vec::new(),
            string_field_ptrs: HashMap::new(),
        }
    }

    pub fn compile(&mut self, ir_module: &IrModule) -> Result<*const u8, XuloError> {
        // 嵌入字符串到 JIT 内存
        self.string_ptrs.clear();
        for s in ir_module.strings.iter() {
            let c_string = std::ffi::CString::new(s.as_str())
                .map_err(|e| XuloError::new(ErrorKind::Codegen, format!("invalid string: {}", e)))?;
            let ptr = c_string.into_raw() as *const u8;
            self.string_ptrs.push(ptr);
        }

        // 预计算所有字段名字符串指针
        self.string_field_ptrs.clear();
        for func in &ir_module.functions {
            for instr in &func.instructions {
                match instr {
                    Instruction::GetField { field, .. } | Instruction::SetField { field, .. } => {
                        if !self.string_field_ptrs.contains_key(field) {
                            let ptr = self.get_or_create_string_ptr(field);
                            self.string_field_ptrs.insert(field.clone(), ptr);
                        }
                    }
                    Instruction::NewObject { fields, .. } => {
                        for (field_name, _, _) in fields {
                            if !self.string_field_ptrs.contains_key(field_name) {
                                let ptr = self.get_or_create_string_ptr(field_name);
                                self.string_field_ptrs.insert(field_name.clone(), ptr);
                            }
                        }
                    }
                    Instruction::Nop => {}
                    _ => {}
                }
            }
        }

        let mut func_ids = Vec::new();

        // 1. 声明所有函数签名
        for func in &ir_module.functions {
            let sig = self.create_signature(func);
            let func_id = self.module
                .declare_function(&func.name, Linkage::Export, &sig)
                .map_err(|e| XuloError::new(ErrorKind::Codegen, format!("failed to declare function: {}", e)))?;
            func_ids.push(func_id);
        }

        // 2. 声明运行时函数
        self.declare_runtime_functions()?;

        // 3. 编译所有函数体
        for (i, func) in ir_module.functions.iter().enumerate() {
            self.compile_function(func, func_ids[i], &func_ids)?;
        }

        // 4. Finalize
        self.module.finalize_definitions()
            .map_err(|e| XuloError::new(ErrorKind::Codegen, format!("failed to finalize: {}", e)))?;

        let entry_id = func_ids[ir_module.entry_point.0 as usize];
        let entry_fn = self.module.get_finalized_function(entry_id);
        Ok(entry_fn)
    }

    fn declare_runtime_functions(&mut self) -> Result<(), XuloError> {
        let rt = [
            ("xulo_print", vec![I64], true),
            ("xulo_print_int", vec![I64], true),
            ("xulo_print_float", vec![cranelift::codegen::ir::types::F64], true),
            ("xulo_print_value", vec![I64, I64], false),
            ("xulo_panic", vec![I64], true),
            ("xulo_string_concat", vec![I64, I64], true),
            ("xulo_to_string", vec![I64], true),
            ("xulo_alloc_object", vec![I64], true),
            ("xulo_alloc_array", vec![I64], true),
            ("xulo_array_push", vec![I64, I64, I64], false),
            ("xulo_array_len", vec![I64], true),
            ("xulo_array_get", vec![I64, I64], true),
            ("xulo_array_set", vec![I64, I64, I64, I64], false),
            ("xulo_array_concat", vec![I64, I64], true),
            ("xulo_object_get", vec![I64, I64], true),
            ("xulo_object_set", vec![I64, I64, I64, I64], false),
            ("xulo_register_float", vec![I64], false),
            ("xulo_register_string", vec![I64], false),
        ];

        for (name, param_types, has_return) in rt {
            let mut sig = Signature::new(CallConv::SystemV);
            for pt in param_types {
                sig.params.push(cranelift::codegen::ir::AbiParam::new(pt));
            }
            if has_return {
                sig.returns.push(cranelift::codegen::ir::AbiParam::new(I64));
            }
            let id = self.module.declare_function(name, Linkage::Import, &sig)
                .map_err(|e| XuloError::new(ErrorKind::Codegen, format!("{}: {}", name, e)))?;
            self.runtime_func_ids.insert(name.to_string(), id);
        }

        Ok(())
    }

    fn create_signature(&self, func: &IrFunction) -> Signature {
        let mut sig = Signature::new(CallConv::SystemV);
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
        let mut ctx = self.module.make_context();
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
            let fref = self.module.declare_func_in_func(reg_id, &mut builder.func);
            for ptr in &self.string_ptrs {
                let ptr_val = builder.ins().iconst(I64, *ptr as i64);
                builder.ins().call(fref, &[ptr_val]);
            }
        }

        for label in &ir_func.labels {
            let block = builder.create_block();
            block_map.insert(*label, block);
        }

        // 声明所有变量的类型
        for (idx, (local_id, _ty)) in ir_func.locals.iter().enumerate() {
            if let Some(&var) = vars.get(local_id) {
                if idx < var_types.len() {
                    builder.declare_var(var, var_types[idx]);
                }
            }
        }

        // 声明参数变量
        for (i, (local_id, _)) in ir_func.params.iter().enumerate() {
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
                            if let Some(&reg_id) = self.runtime_func_ids.get("xulo_register_float") {
                                let fref = self.module.declare_func_in_func(reg_id, &mut builder.func);
                                let bits_val = builder.ins().iconst(I64, bits);
                                builder.ins().call(fref, &[bits_val]);
                            }
                            v
                        }
                        IrValue::Null => builder.ins().iconst(I64, 0),
                        IrValue::String(idx) => {
                            if let Some(&ptr) = self.string_ptrs.get(*idx) {
                                builder.ins().iconst(I64, ptr as i64)
                            } else {
                                builder.ins().iconst(I64, 0)
                            }
                        }
                    };
                    if let Some(&var) = vars.get(dst) {
                        builder.def_var(var, val);
                    }
                }
                Instruction::Add { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().iadd(l, r);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Sub { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().isub(l, r);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Mul { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().imul(l, r);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Div { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().sdiv(l, r);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Neg { dst, operand } => {
                    let o = resolve_operand(&mut builder, operand, &vars);
                    let val = builder.ins().ineg(o);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Eq { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let cmp = builder.ins().icmp(IntCC::Equal, l, r);
                    let val = builder.ins().uextend(I64, cmp);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Neq { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let cmp = builder.ins().icmp(IntCC::NotEqual, l, r);
                    let val = builder.ins().uextend(I64, cmp);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Lt { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let cmp = builder.ins().icmp(IntCC::SignedLessThan, l, r);
                    let val = builder.ins().uextend(I64, cmp);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Gt { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let cmp = builder.ins().icmp(IntCC::SignedGreaterThan, l, r);
                    let val = builder.ins().uextend(I64, cmp);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Lte { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let cmp = builder.ins().icmp(IntCC::SignedLessThanOrEqual, l, r);
                    let val = builder.ins().uextend(I64, cmp);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Gte { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let cmp = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, l, r);
                    let val = builder.ins().uextend(I64, cmp);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::And { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().band(l, r);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Or { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().bor(l, r);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Not { dst, operand } => {
                    let o = resolve_operand(&mut builder, operand, &vars);
                    let val = builder.ins().bnot(o);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Branch { condition, then_label, else_label } => {
                    let cond = resolve_operand(&mut builder, condition, &vars);
                    let cond_bool = builder.ins().icmp_imm(cranelift::codegen::ir::condcodes::IntCC::NotEqual, cond, 0);
                    let then_block = block_map[then_label];
                    let else_block = block_map[else_label];
                    builder.ins().brif(cond_bool, then_block, &[], else_block, &[]);
                }
                Instruction::Jump(label) => {
                    let block = block_map[label];
                    builder.ins().jump(block, &[]);
                }
                Instruction::Return(value) => {
                    if let Some(v) = value {
                        let ret_val = resolve_operand(&mut builder, v, &vars);
                        builder.ins().return_(&[ret_val]);
                    } else {
                        builder.ins().return_(&[]);
                    }
                }
                Instruction::SetLocal { dst, src } => {
                    let val = resolve_operand(&mut builder, src, &vars);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::GetLocal { dst, src } => {
                    if let Some(&src_var) = vars.get(src) {
                        let val = builder.use_var(src_var);
                        if let Some(&dst_var) = vars.get(dst) { builder.def_var(dst_var, val); }
                    }
                }
                Instruction::RuntimeCall { dst, func, args } => {
                    let arg_vals: Vec<_> = args.iter()
                        .map(|a| resolve_operand(&mut builder, a, &vars))
                        .collect();

                    let fn_name = match func {
                        RuntimeFn::Print => "xulo_print",
                        RuntimeFn::PrintInt => "xulo_print_int",
                        RuntimeFn::PrintFloat => "xulo_print_float",
                        RuntimeFn::PrintValue => "xulo_print_value",
                        RuntimeFn::Panic => "xulo_panic",
                        RuntimeFn::StringConcat => "xulo_string_concat",
                        RuntimeFn::ToString => "xulo_to_string",
                        RuntimeFn::AllocObject => "xulo_alloc_object",
                        RuntimeFn::AllocArray => "xulo_alloc_array",
                        RuntimeFn::ArrayPush => "xulo_array_push",
                        RuntimeFn::ArrayLen => "xulo_array_len",
                        RuntimeFn::ArrayGet => "xulo_array_get",
                        RuntimeFn::ArrayGetTag => "xulo_array_get_tag",
                        RuntimeFn::ArraySet => "xulo_array_set",
                        RuntimeFn::ArrayConcat => "xulo_array_concat",
                        RuntimeFn::ObjectGet => "xulo_object_get",
                        RuntimeFn::ObjectSet => "xulo_object_set",
                    };

                    // xulo_print_value 期望 (i64, i64)，需要将 f64 参数 bitcast 到 i64
                    let final_args = if *func == RuntimeFn::PrintValue && arg_vals.len() >= 1 {
                        let val_ty = builder.func.dfg.value_type(arg_vals[0]);
                        if val_ty == cranelift::codegen::ir::types::F64 {
                            let bitcasted = builder.ins().bitcast(I64, cranelift::codegen::ir::MemFlags::new(), arg_vals[0]);
                            let mut new_args = vec![bitcasted];
                            new_args.extend_from_slice(&arg_vals[1..]);
                            new_args
                        } else {
                            arg_vals
                        }
                    } else {
                        arg_vals
                    };

                    if let Some(&func_id) = self.runtime_func_ids.get(fn_name) {
                        let fref = self.module.declare_func_in_func(func_id, &mut builder.func);
                        let call = builder.ins().call(fref, &final_args);
                        if let Some(d) = dst {
                            if let Some(&var) = vars.get(d) {
                                if let Some(result) = builder.inst_results(call).first() {
                                    builder.def_var(var, *result);
                                }
                            }
                        }
                    }
                }
                Instruction::Call { dst, func, args } => {
                    let arg_vals: Vec<_> = args.iter()
                        .map(|a| resolve_operand(&mut builder, a, &vars))
                        .collect();

                    if let Some(call_id) = all_func_ids.get(func.0) {
                        let func_ref = self.module.declare_func_in_func(*call_id, &mut builder.func);
                        let call = builder.ins().call(func_ref, &arg_vals);

                        if let Some(d) = dst {
                            if let Some(&var) = vars.get(d) {
                                if let Some(result) = builder.inst_results(call).first() {
                                    builder.def_var(var, *result);
                                }
                            }
                        }
                    }
                }
                Instruction::NewObject { dst, fields } => {
                    let size_val = builder.ins().iconst(I64, 0);
                    if let Some(&func_id) = self.runtime_func_ids.get("xulo_alloc_object") {
                        let fref = self.module.declare_func_in_func(func_id, &mut builder.func);
                        let call = builder.ins().call(fref, &[size_val]);
                        if let Some(&var) = vars.get(dst) {
                            if let Some(result) = builder.inst_results(call).first() {
                                builder.def_var(var, *result);
                                for (field_name, field_val, tag) in fields.iter() {
                                    let obj_val = builder.use_var(var);
                                    let val = resolve_operand(&mut builder, field_val, &vars);
                                    let tag_val = builder.ins().iconst(I64, *tag);
                                    if let Some(&field_ptr) = self.string_field_ptrs.get(field_name) {
                                        let field_v = builder.ins().iconst(I64, field_ptr as i64);
                                        if let Some(&set_id) = self.runtime_func_ids.get("xulo_object_set") {
                                            let fref2 = self.module.declare_func_in_func(set_id, &mut builder.func);
                                            builder.ins().call(fref2, &[obj_val, field_v, val, tag_val]);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Instruction::NewArray { dst, elements } => {
                    let size_val = builder.ins().iconst(I64, 0);
                    if let Some(&func_id) = self.runtime_func_ids.get("xulo_alloc_array") {
                        let fref = self.module.declare_func_in_func(func_id, &mut builder.func);
                        let call = builder.ins().call(fref, &[size_val]);
                        if let Some(&var) = vars.get(dst) {
                            if let Some(result) = builder.inst_results(call).first() {
                                builder.def_var(var, *result);
                                for (elem, tag) in elements.iter() {
                                    let arr_val = builder.use_var(var);
                                    let elem_val = resolve_operand(&mut builder, elem, &vars);
                                    let tag_val = builder.ins().iconst(I64, *tag);
                                    if let Some(&push_id) = self.runtime_func_ids.get("xulo_array_push") {
                                        let fref2 = self.module.declare_func_in_func(push_id, &mut builder.func);
                                        builder.ins().call(fref2, &[arr_val, elem_val, tag_val]);
                                    }
                                }
                            }
                        }
                    }
                }
                Instruction::GetField { dst, object, field } => {
                    let obj_val = resolve_operand(&mut builder, object, &vars);
                    if let Some(&field_ptr) = self.string_field_ptrs.get(field) {
                        let field_val = builder.ins().iconst(I64, field_ptr as i64);
                        if let Some(&func_id) = self.runtime_func_ids.get("xulo_object_get") {
                            let fref = self.module.declare_func_in_func(func_id, &mut builder.func);
                            let call = builder.ins().call(fref, &[obj_val, field_val]);
                            if let Some(&var) = vars.get(dst) {
                                if let Some(result) = builder.inst_results(call).first() {
                                    builder.def_var(var, *result);
                                }
                            }
                        }
                    }
                }
                Instruction::SetField { object, field, value, tag } => {
                    let obj_val = resolve_operand(&mut builder, object, &vars);
                    if let Some(&field_ptr) = self.string_field_ptrs.get(field) {
                        let field_val = builder.ins().iconst(I64, field_ptr as i64);
                        let val = resolve_operand(&mut builder, value, &vars);
                        let tag_val = builder.ins().iconst(I64, *tag);
                        if let Some(&func_id) = self.runtime_func_ids.get("xulo_object_set") {
                            let fref = self.module.declare_func_in_func(func_id, &mut builder.func);
                            builder.ins().call(fref, &[obj_val, field_val, val, tag_val]);
                        }
                    }
                }
                Instruction::GetIndex { dst, array, index } => {
                    let arr_val = resolve_operand(&mut builder, array, &vars);
                    let idx_val = resolve_operand(&mut builder, index, &vars);
                    if let Some(&func_id) = self.runtime_func_ids.get("xulo_array_get") {
                        let fref = self.module.declare_func_in_func(func_id, &mut builder.func);
                        let call = builder.ins().call(fref, &[arr_val, idx_val]);
                        if let Some(&var) = vars.get(dst) {
                            if let Some(result) = builder.inst_results(call).first() {
                                builder.def_var(var, *result);
                            }
                        }
                    }
                }
                Instruction::SetIndex { array, index, value, tag } => {
                    let arr_val = resolve_operand(&mut builder, array, &vars);
                    let idx_val = resolve_operand(&mut builder, index, &vars);
                    let val = resolve_operand(&mut builder, value, &vars);
                    let tag_val = builder.ins().iconst(I64, *tag);
                    if let Some(&func_id) = self.runtime_func_ids.get("xulo_array_set") {
                        let fref = self.module.declare_func_in_func(func_id, &mut builder.func);
                        builder.ins().call(fref, &[arr_val, idx_val, val, tag_val]);
                    }
                }
                _ => {}
            }
        }

        builder.seal_block(entry_block);
        builder.seal_all_blocks();
        builder.finalize();

        self.module.define_function(func_id, &mut ctx)
            .map_err(|e| XuloError::new(ErrorKind::Codegen, format!("failed to define function: {}", e)))?;

        Ok(())
    }

    fn get_or_create_string_ptr(&mut self, s: &str) -> *const u8 {
        // Check if string already exists
        for (_i, ptr) in self.string_ptrs.iter().enumerate() {
            unsafe {
                let c_str = std::ffi::CStr::from_ptr(*ptr as *const std::os::raw::c_char);
                if let Ok(existing) = c_str.to_str() {
                    if existing == s {
                        return *ptr;
                    }
                }
            }
        }

        // Create new string
        let c_string = std::ffi::CString::new(s).expect("failed to create CString");
        let ptr = c_string.into_raw() as *const u8;
        self.string_ptrs.push(ptr);
        ptr
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
