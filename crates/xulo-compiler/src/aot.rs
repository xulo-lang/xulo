use std::collections::HashMap;

use cranelift::codegen::ir::types::I64;
use cranelift::codegen::ir::{Signature, InstBuilder};
use cranelift::codegen::ir::condcodes::*;
use cranelift::frontend::{FunctionBuilder, Variable, FunctionBuilderContext};
use cranelift::codegen::isa::CallConv;
use cranelift::codegen::settings;
use cranelift::codegen::verify_function;
use cranelift_object::{ObjectBuilder, ObjectModule, ObjectProduct};
use cranelift_module::{Module, FuncId, Linkage, DataId, DataDescription};
use cranelift::codegen::ir::entities::Block;

use xulo_core::ir::*;
use xulo_core::error::{XuloError, ErrorKind};

fn block_has_terminator(func: &cranelift::codegen::ir::Function, block: Block) -> bool {
    if let Some(inst) = func.layout.last_inst(block) {
        func.dfg.insts[inst].opcode().is_terminator()
    } else {
        false
    }
}

pub struct AotCodeGen {
    module: ObjectModule,
    builder_ctx: FunctionBuilderContext,
    runtime_func_ids: HashMap<String, FuncId>,
    string_data_ids: Vec<DataId>,
    string_field_ids: HashMap<String, DataId>,
}

impl AotCodeGen {
    pub fn new() -> Result<Self, XuloError> {
        let isa_builder = cranelift_native::builder()
            .map_err(|e| XuloError::new(ErrorKind::Codegen, format!("failed to get ISA builder: {}", e)))?;

        let flag_builder = settings::builder();
        let isa = isa_builder.finish(settings::Flags::new(flag_builder))
            .map_err(|e| XuloError::new(ErrorKind::Codegen, format!("failed to create ISA: {}", e)))?;

        let obj_builder = ObjectBuilder::new(
            isa,
            "xulo_program".to_string(),
            cranelift_module::default_libcall_names(),
        )
        .map_err(|e| XuloError::new(ErrorKind::Codegen, format!("failed to create ObjectBuilder: {}", e)))?;

        let module = ObjectModule::new(obj_builder);

        Ok(AotCodeGen {
            module,
            builder_ctx: FunctionBuilderContext::new(),
            runtime_func_ids: HashMap::new(),
            string_data_ids: Vec::new(),
            string_field_ids: HashMap::new(),
        })
    }

    pub fn compile(mut self, ir_module: &IrModule) -> Result<ObjectProduct, XuloError> {
        // Embed strings into .rodata
        self.string_data_ids.clear();
        for s in ir_module.strings.iter() {
            let data_id = self.module
                .declare_data(&format!("str_{}", self.string_data_ids.len()), Linkage::Local, false, false)
                .map_err(|e| XuloError::new(ErrorKind::Codegen, format!("failed to declare string data: {}", e)))?;

            let mut data_desc = DataDescription::new();
            let mut bytes = s.as_bytes().to_vec();
            bytes.push(0); // null terminator
            data_desc.define(bytes.into_boxed_slice());
            self.module.define_data(data_id, &mut data_desc)
                .map_err(|e| XuloError::new(ErrorKind::Codegen, format!("failed to define string data: {}", e)))?;

            self.string_data_ids.push(data_id);
        }

        // Pre-compute field name string data_ids
        self.string_field_ids.clear();
        for func in &ir_module.functions {
            for instr in &func.instructions {
                match instr {
                    Instruction::GetField { field, .. } | Instruction::SetField { field, .. } => {
                        if !self.string_field_ids.contains_key(field) {
                            let data_id = self.get_or_create_string_id(field);
                            self.string_field_ids.insert(field.clone(), data_id);
                        }
                    }
                    Instruction::NewObject { fields, .. } => {
                        for (field_name, _, _) in fields.iter() {
                            if !self.string_field_ids.contains_key(field_name) {
                                let data_id = self.get_or_create_string_id(field_name);
                                self.string_field_ids.insert(field_name.clone(), data_id);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Declare all user functions as Export
        let mut func_ids = Vec::new();
        for func in &ir_module.functions {
            let sig = self.create_signature(func);
            let func_id = self.module
                .declare_function(&func.name, Linkage::Export, &sig)
                .map_err(|e| XuloError::new(ErrorKind::Codegen, format!("failed to declare function {}: {}", func.name, e)))?;
            func_ids.push(func_id);
        }

        // Declare runtime functions
        self.declare_runtime_functions()?;

        // Compile all function bodies
        for (i, func) in ir_module.functions.iter().enumerate() {
            self.compile_function(func, func_ids[i], &func_ids)?;
        }

        Ok(self.module.finish())
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
        builder.func.layout.append_block(entry_block);
        builder.switch_to_block(entry_block);

        // 注册所有字符串指针到运行时（用于自动类型检测）
        if let Some(&reg_id) = self.runtime_func_ids.get("xulo_register_string") {
            let fref = self.module.declare_func_in_func(reg_id, &mut builder.func);
            for data_id in &self.string_data_ids {
                let gv = self.module.declare_data_in_func(*data_id, &mut builder.func);
                let ptr_val = builder.ins().global_value(I64, gv);
                builder.ins().call(fref, &[ptr_val]);
            }
        }

        for label in &ir_func.labels {
            let block = builder.create_block();
            block_map.insert(*label, block);
            builder.func.layout.append_block(block);
        }

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
                        // If previous block is now empty, add dummy instruction
                        if let Some(cur) = builder.current_block() {
                            if builder.func.layout.last_inst(cur).is_none()
                                && cur != entry_block
                            {
                                builder.ins().iconst(I64, 0);
                            }
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
                            if let Some(&reg_id) = self.runtime_func_ids.get("xulo_register_float") {
                                let fref = self.module.declare_func_in_func(reg_id, &mut builder.func);
                                let bits_val = builder.ins().iconst(I64, bits);
                                builder.ins().call(fref, &[bits_val]);
                            }
                            v
                        }
                        IrValue::Null => builder.ins().iconst(I64, 0),
                        IrValue::String(idx) => {
                            if let Some(&data_id) = self.string_data_ids.get(*idx) {
                                let gv = self.module.declare_data_in_func(data_id, &mut builder.func);
                                builder.ins().global_value(I64, gv)
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
                Instruction::Mod { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().srem(l, r);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Pow { .. } => {
                    // Pow not natively supported in Cranelift; handled by runtime interpreter
                }
                Instruction::Neg { dst, operand } => {
                    let o = resolve_operand(&mut builder, operand, &vars);
                    let val = builder.ins().ineg(o);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::BitAnd { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().band(l, r);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::BitOr { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().bor(l, r);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Xor { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().bxor(l, r);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Shl { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().ishl(l, r);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::Shr { dst, left, right } => {
                    let l = resolve_operand(&mut builder, left, &vars);
                    let r = resolve_operand(&mut builder, right, &vars);
                    let val = builder.ins().sshr(l, r);
                    if let Some(&var) = vars.get(dst) { builder.def_var(var, val); }
                }
                Instruction::BitNot { dst, operand } => {
                    let o = resolve_operand(&mut builder, operand, &vars);
                    let val = builder.ins().bnot(o);
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
                    let cond_bool = builder.ins().icmp_imm(IntCC::NotEqual, cond, 0);
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
                                    if let Some(&data_id) = self.string_field_ids.get(field_name) {
                                        let gv = self.module.declare_data_in_func(data_id, &mut builder.func);
                                        let field_ptr = builder.ins().global_value(I64, gv);
                                        if let Some(&set_id) = self.runtime_func_ids.get("xulo_object_set") {
                                            let fref2 = self.module.declare_func_in_func(set_id, &mut builder.func);
                                            builder.ins().call(fref2, &[obj_val, field_ptr, val, tag_val]);
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
                    if let Some(&data_id) = self.string_field_ids.get(field) {
                        let gv = self.module.declare_data_in_func(data_id, &mut builder.func);
                        let field_val = builder.ins().global_value(I64, gv);
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
                    let val = resolve_operand(&mut builder, value, &vars);
                    let tag_val = builder.ins().iconst(I64, *tag);
                    if let Some(&data_id) = self.string_field_ids.get(field) {
                        let gv = self.module.declare_data_in_func(data_id, &mut builder.func);
                        let field_val = builder.ins().global_value(I64, gv);
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
                Instruction::Nop => {}
            }
        }

        // Ensure ALL blocks have terminators
        // First, ensure current block (last label block) has a terminator
        if let Some(current_block) = builder.current_block() {
            if !block_has_terminator(&builder.func, current_block) {
                if !ir_func.return_type.is_void() {
                    let zero = builder.ins().iconst(I64, 0);
                    builder.ins().return_(&[zero]);
                } else {
                    builder.ins().return_(&[]);
                }
            }
        }
        // Also ensure entry_block has a terminator (it may have been switched away from)
        if !block_has_terminator(&builder.func, entry_block) {
            if let Some(&first_label) = ir_func.labels.first() {
                if let Some(&first_block) = block_map.get(&first_label) {
                    builder.switch_to_block(entry_block);
                    builder.ins().jump(first_block, &[]);
                }
            } else {
                builder.switch_to_block(entry_block);
                if !ir_func.return_type.is_void() {
                    let zero = builder.ins().iconst(I64, 0);
                    builder.ins().return_(&[zero]);
                } else {
                    builder.ins().return_(&[]);
                }
            }
        }

        builder.seal_all_blocks();
        builder.finalize();

        // Run verifier to get detailed error messages
        if let Err(errors) = verify_function(&ctx.func, self.module.isa()) {
            eprintln!("Verifier errors for {}:\n{}", ir_func.name, errors);
            return Err(XuloError::new(ErrorKind::Codegen, format!("verifier errors for {}: {}", ir_func.name, errors)));
        }

        self.module.define_function(func_id, &mut ctx)
            .map_err(|e| XuloError::new(ErrorKind::Codegen, format!("failed to define function {}: {}", ir_func.name, e)))?;

        Ok(())
    }

    fn get_or_create_string_id(&mut self, s: &str) -> DataId {
        if let Some(&data_id) = self.string_field_ids.get(s) {
            return data_id;
        }

        let data_id = self.module
            .declare_data(&format!("str_{}", self.string_data_ids.len()), Linkage::Local, false, false)
            .expect("failed to declare string data");

        let mut data_desc = DataDescription::new();
        let mut bytes = s.as_bytes().to_vec();
        bytes.push(0); // null terminator
        data_desc.define(bytes.into_boxed_slice());
        self.module.define_data(data_id, &mut data_desc)
            .expect("failed to define string data");

        self.string_data_ids.push(data_id);
        self.string_field_ids.insert(s.to_string(), data_id);
        data_id
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
