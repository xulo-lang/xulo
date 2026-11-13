//! 运行时/外部函数声明与调用边界转换。

use xulo_core::ir::{FuncId, RuntimeFn};
use xulo_runtime::runtime::{
    BUILDER_CAP_OFFSET, BUILDER_LEN_OFFSET, BUILDER_PTR_OFFSET, BUILDER_SCALARS_OFFSET,
};

use crate::codegen::runtime_fn_name;

use super::*;

impl<'a, 'ctx> Emitter<'a, 'ctx> {
    pub(crate) fn declare_runtime_functions(&mut self) -> Result<(), XuloError> {
        for desc in RUNTIME_FNS {
            let params: Vec<BasicTypeEnum<'ctx>> = desc
                .params
                .iter()
                .map(|t| abi::runtime_type(self.context, *t))
                .collect();
            let returns = desc
                .returns
                .first()
                .map(|t| abi::runtime_type(self.context, *t));
            let fn_ty = abi::function_type(self.context, &params, returns);
            let fv = self
                .module
                .add_function(desc.name, fn_ty, Some(Linkage::External));
            self.runtime.insert(
                desc.name.to_string(),
                RuntimeSig {
                    fv,
                    params: desc.params.iter().map(|t| runtime_kind(*t)).collect(),
                    ret: desc.returns.first().map(|t| runtime_kind(*t)),
                },
            );
        }
        Ok(())
    }

    pub(crate) fn declare_externs(&mut self) -> Result<(), XuloError> {
        for ext in &self.ir.externs {
            let params: Vec<BasicTypeEnum<'ctx>> = ext
                .params
                .iter()
                .map(|p| abi::abi_type(self.context, *p))
                .collect();
            let returns = ext.return_type.map(|r| abi::abi_type(self.context, r));
            let fn_ty = abi::function_type(self.context, &params, returns);
            let fv = self
                .module
                .add_function(&ext.symbol, fn_ty, Some(Linkage::External));
            self.externs.push(fv);
        }
        Ok(())
    }

    pub(crate) fn coerce_extern_arg(
        &mut self,
        value: BasicValueEnum<'ctx>,
        param: IrAbiParam,
    ) -> Result<BasicValueEnum<'ctx>, XuloError> {
        let want = abi::abi_type(self.context, param);
        let actual = value.get_type();
        if actual == want {
            return Ok(value);
        }
        match param {
            IrAbiParam::F32 | IrAbiParam::F64 => {
                let f64v = if value.is_int_value() {
                    cg(self.builder.build_signed_int_to_float(
                        value.into_int_value(),
                        self.context.f64_type(),
                        "ffi",
                    ))?
                    .into()
                } else if self.is_f32(value) {
                    cg(self.builder.build_float_ext(
                        value.into_float_value(),
                        self.context.f64_type(),
                        "ffi",
                    ))?
                    .into()
                } else {
                    value
                };
                if param == IrAbiParam::F32 {
                    Ok(cg(self.builder.build_float_trunc(
                        f64v.into_float_value(),
                        self.context.f32_type(),
                        "ffi",
                    ))?
                    .into())
                } else {
                    Ok(f64v)
                }
            }
            _ => {
                let wide = if value.is_float_value() {
                    let f = if self.is_f32(value) {
                        cg(self.builder.build_float_ext(
                            value.into_float_value(),
                            self.context.f64_type(),
                            "ffi",
                        ))?
                    } else {
                        value.into_float_value()
                    };
                    self.float_to_i64_trapping(f)?
                } else if value.is_int_value() {
                    let i = value.into_int_value();
                    if i.get_type().get_bit_width() < 64 {
                        cg(self
                            .builder
                            .build_int_z_extend(i, self.context.i64_type(), "ffi"))?
                    } else {
                        i
                    }
                } else {
                    return Ok(value);
                };
                if actual == want {
                    Ok(wide.into())
                } else {
                    Ok(cg(self
                        .builder
                        .build_int_truncate(wide, want.into_int_type(), "ffi"))?
                    .into())
                }
            }
        }
    }

    pub(crate) fn coerce_extern_ret(
        &self,
        value: BasicValueEnum<'ctx>,
        ret: IrAbiParam,
    ) -> Result<BasicValueEnum<'ctx>, XuloError> {
        match ret {
            IrAbiParam::S8 | IrAbiParam::S16 | IrAbiParam::S32 => {
                if value.is_int_value() && value.into_int_value().get_type().get_bit_width() < 64 {
                    Ok(cg(self.builder.build_int_s_extend(
                        value.into_int_value(),
                        self.context.i64_type(),
                        "ffi",
                    ))?
                    .into())
                } else {
                    Ok(value)
                }
            }
            IrAbiParam::Bool | IrAbiParam::U8 | IrAbiParam::U16 | IrAbiParam::U32 => {
                if value.is_int_value() && value.into_int_value().get_type().get_bit_width() < 64 {
                    Ok(cg(self.builder.build_int_z_extend(
                        value.into_int_value(),
                        self.context.i64_type(),
                        "ffi",
                    ))?
                    .into())
                } else {
                    Ok(value)
                }
            }
            IrAbiParam::S64 | IrAbiParam::U64 | IrAbiParam::F64 => Ok(value),
            IrAbiParam::F32 => {
                if self.is_f32(value) {
                    Ok(cg(self.builder.build_float_ext(
                        value.into_float_value(),
                        self.context.f64_type(),
                        "ffi",
                    ))?
                    .into())
                } else {
                    Ok(value)
                }
            }
        }
    }

    pub(crate) fn call_runtime(
        &mut self,
        name: &str,
        args: &[BasicValueEnum<'ctx>],
    ) -> Result<Option<BasicValueEnum<'ctx>>, XuloError> {
        let sig =
            self.runtime.get(name).cloned().ok_or_else(|| {
                codegen_err(format!("runtime function `{name}` was not declared"))
            })?;
        let mut arg_vals: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(args.len());
        for (i, arg) in args.iter().enumerate() {
            let want = sig.params.get(i).copied().unwrap_or(RuntimeKind::I64);
            let kind = match want {
                RuntimeKind::I64 => SlotKind::I64,
                RuntimeKind::F64 => SlotKind::F64,
            };
            let coerced = self.coerce_to(*arg, kind)?;
            arg_vals.push(coerced.into());
        }
        let call = cg(self.builder.build_call(sig.fv, &arg_vals, "rt"))?;
        match (sig.ret, call.try_as_basic_value().basic()) {
            (Some(_), Some(value)) => Ok(Some(value)),
            _ => Ok(None),
        }
    }

    /// `std::hint::black_box`: `"" : "=r,0"` / `"" : "=x,0"` — an empty
    /// side-effecting asm block whose output is tied to its input. The value
    /// is copied through an opaque register, so the optimizer cannot see
    /// through it, and the machine code is just a register move.
    fn lower_black_box(
        &mut self,
        dst: &Option<LocalId>,
        args: &[Operand],
    ) -> Result<(), XuloError> {
        let arg = self.resolve(&args[0])?;
        let kind = dst
            .as_ref()
            .and_then(|d| self.local_slot_kind(d))
            .unwrap_or_else(|| {
                if self.is_f64(arg) {
                    SlotKind::F64
                } else {
                    SlotKind::I64
                }
            });
        let (ty, constraints) = match kind {
            SlotKind::F64 => (
                self.context
                    .f64_type()
                    .fn_type(&[self.context.f64_type().into()], false),
                "=x,0",
            ),
            SlotKind::I64 => (
                self.context
                    .i64_type()
                    .fn_type(&[self.context.i64_type().into()], false),
                "=r,0",
            ),
        };
        let value = self.coerce_to(arg, kind)?;
        let asm = self.context.create_inline_asm(
            ty,
            String::new(),
            constraints.to_string(),
            true,
            false,
            None,
            false,
        );
        let call = cg(self
            .builder
            .build_indirect_call(ty, asm, &[value.into()], "black_box"))?;
        let result = call
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| codegen_err("inline asm produced no value"))?;
        if let Some(dst) = dst {
            self.store(dst, result, StoreMode::Coerce)?;
        }
        Ok(())
    }

    pub(crate) fn lower_runtime_call(
        &mut self,
        dst: &Option<LocalId>,
        func: &RuntimeFn,
        args: &[Operand],
    ) -> Result<(), XuloError> {
        let name = runtime_fn_name(func);
        if matches!(func, RuntimeFn::StringBuilderAppendKnown) {
            return self.lower_string_builder_append_known(dst, args);
        }
        // `std::hint::black_box` lowers to an inline-asm identity barrier
        // (the same trick Rust's `black_box` uses) instead of a runtime call:
        // no call overhead, but the optimizer can never fold the value.
        if matches!(name, "xulo_hint_black_box" | "xulo_hint_black_box_f64") {
            return self.lower_black_box(dst, args);
        }
        let mut arg_vals = Vec::with_capacity(args.len());
        for arg in args {
            arg_vals.push(self.resolve(arg)?);
        }
        let result = self.call_runtime(name, &arg_vals)?;
        if let (Some(dst), Some(result)) = (dst, result) {
            self.store(dst, result, StoreMode::Coerce)?;
        }
        Ok(())
    }

    /// `StringBuilder::append` 的 AOT 快路径：容量足够时在调用点直接写入
    /// builder 的裸缓冲区，避免 AOT -> runtime 的函数调用；容量不足或句柄/
    /// 文本为空时回落到 `xulo_string_builder_append_known`。
    fn lower_string_builder_append_known(
        &mut self,
        dst: &Option<LocalId>,
        args: &[Operand],
    ) -> Result<(), XuloError> {
        let handle = self.resolve_i64(&args[0])?;
        let text = self.resolve_i64(&args[1])?;
        let byte_len = self.resolve_i64(&args[2])?;
        let scalars = self.resolve_i64(&args[3])?;

        let i64_ty = self.context.i64_type();
        let i8_ty = self.context.i8_type();
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
        let zero = i64_ty.const_zero();

        let fv = self
            .cur_fn
            .ok_or_else(|| codegen_err("StringBuilder append outside a function"))?;
        let check_bb = self.context.append_basic_block(fv, "sb.check");
        let fast_bb = self.context.append_basic_block(fv, "sb.fast");
        let slow_bb = self.context.append_basic_block(fv, "sb.slow");
        let done_bb = self.context.append_basic_block(fv, "sb.done");

        let handle_ptr = cg(self.builder.build_int_to_ptr(handle, ptr_ty, "sb.handle"))?;
        let text_ptr = cg(self.builder.build_int_to_ptr(text, ptr_ty, "sb.text"))?;

        // 空句柄/空文本直接走运行时；字段访问必须放在这个检查之后。
        let handle_ok =
            cg(self
                .builder
                .build_int_compare(IntPredicate::NE, handle, zero, "sb.handle_ok"))?;
        let text_ok =
            cg(self
                .builder
                .build_int_compare(IntPredicate::NE, text, zero, "sb.text_ok"))?;
        let len_ok =
            cg(self
                .builder
                .build_int_compare(IntPredicate::SGT, byte_len, zero, "sb.len_ok"))?;
        let scalars_ok =
            cg(self
                .builder
                .build_int_compare(IntPredicate::SGE, scalars, zero, "sb.scalars_ok"))?;
        let safe = cg(self.builder.build_and(handle_ok, text_ok, "sb.safe"))?;
        let safe = cg(self.builder.build_and(safe, len_ok, "sb.safe"))?;
        let safe = cg(self.builder.build_and(safe, scalars_ok, "sb.safe"))?;
        cg(self
            .builder
            .build_conditional_branch(safe, check_bb, slow_bb))?;

        self.builder.position_at_end(check_bb);
        let len_ptr = unsafe {
            cg(self.builder.build_gep(
                i8_ty,
                handle_ptr,
                &[self.int_const(BUILDER_LEN_OFFSET as i64)],
                "sb.len.ptr",
            ))
        }?;
        let cap_ptr = unsafe {
            cg(self.builder.build_gep(
                i8_ty,
                handle_ptr,
                &[self.int_const(BUILDER_CAP_OFFSET as i64)],
                "sb.cap.ptr",
            ))
        }?;
        let len = cg(self.builder.build_load(i64_ty, len_ptr, "sb.len"))?.into_int_value();
        let cap = cg(self.builder.build_load(i64_ty, cap_ptr, "sb.cap"))?.into_int_value();
        let new_len = cg(self.builder.build_int_add(len, byte_len, "sb.newlen"))?;
        let has_cap =
            cg(self
                .builder
                .build_int_compare(IntPredicate::ULE, new_len, cap, "sb.has_cap"))?;
        cg(self
            .builder
            .build_conditional_branch(has_cap, fast_bb, slow_bb))?;

        self.builder.position_at_end(fast_bb);
        let data_ptr = unsafe {
            cg(self.builder.build_gep(
                i8_ty,
                handle_ptr,
                &[self.int_const(BUILDER_PTR_OFFSET as i64)],
                "sb.data.ptr",
            ))
        }?;
        let data = cg(self.builder.build_load(ptr_ty, data_ptr, "sb.data"))?.into_pointer_value();
        let dest = unsafe { cg(self.builder.build_gep(i8_ty, data, &[len], "sb.dest"))? };
        cg(self.builder.build_memcpy(dest, 1, text_ptr, 1, byte_len))?;
        cg(self.builder.build_store(len_ptr, new_len))?;
        let scalars_ptr = unsafe {
            cg(self.builder.build_gep(
                i8_ty,
                handle_ptr,
                &[self.int_const(BUILDER_SCALARS_OFFSET as i64)],
                "sb.scalars.ptr",
            ))
        }?;
        let old_scalars =
            cg(self.builder.build_load(i64_ty, scalars_ptr, "sb.scalars"))?.into_int_value();
        let new_scalars = cg(self
            .builder
            .build_int_add(old_scalars, scalars, "sb.newscalars"))?;
        cg(self.builder.build_store(scalars_ptr, new_scalars))?;
        cg(self.builder.build_unconditional_branch(done_bb))?;

        self.builder.position_at_end(slow_bb);
        self.call_runtime(
            "xulo_string_builder_append_known",
            &[handle.into(), text.into(), byte_len.into(), scalars.into()],
        )?;
        cg(self.builder.build_unconditional_branch(done_bb))?;

        self.builder.position_at_end(done_bb);
        if let Some(dst) = dst {
            self.store(dst, zero.into(), StoreMode::Coerce)?;
        }
        Ok(())
    }

    pub(crate) fn lower_call(
        &mut self,
        dst: &Option<LocalId>,
        func: FuncId,
        args: &[Operand],
    ) -> Result<(), XuloError> {
        let callee = self.funcs[func.0];
        let mut arg_vals: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(args.len());
        for arg in args {
            let v = self.resolve(arg)?;
            let coerced = self.coerce_to(v, SlotKind::I64)?;
            arg_vals.push(coerced.into());
        }
        let call = cg(self.builder.build_call(callee, &arg_vals, "call"))?;
        if let (Some(dst), Some(result)) = (dst, call.try_as_basic_value().basic()) {
            self.store(dst, result, StoreMode::Coerce)?;
        }
        Ok(())
    }

    pub(crate) fn lower_func_addr(&mut self, dst: &LocalId, func: FuncId) -> Result<(), XuloError> {
        let f = self.funcs[func.0];
        let ptr = f.as_global_value().as_pointer_value();
        let addr = cg(self
            .builder
            .build_ptr_to_int(ptr, self.context.i64_type(), "fnaddr"))?;
        self.store(dst, addr.into(), StoreMode::Coerce)
    }

    pub(crate) fn lower_make_closure(
        &mut self,
        dst: &LocalId,
        func: FuncId,
        env: &Operand,
    ) -> Result<(), XuloError> {
        let env_val = self.resolve(env)?;
        let f = self.funcs[func.0];
        let ptr = f.as_global_value().as_pointer_value();
        let addr = cg(self
            .builder
            .build_ptr_to_int(ptr, self.context.i64_type(), "fnaddr"))?;
        let result = self
            .call_runtime("xulo_alloc_closure", &[addr.into(), env_val])?
            .ok_or_else(|| codegen_err("xulo_alloc_closure unavailable"))?;
        self.store(dst, result, StoreMode::Coerce)
    }

    pub(crate) fn lower_call_indirect(
        &mut self,
        dst: &Option<LocalId>,
        callee: &Operand,
        args: &[Operand],
    ) -> Result<(), XuloError> {
        let callee_val = self.resolve(callee)?;
        let checked = self
            .call_runtime("xulo_check_callable", &[callee_val])?
            .ok_or_else(|| codegen_err("xulo_check_callable unavailable"))?;
        let func_addr = self
            .call_runtime("xulo_closure_func", &[checked])?
            .ok_or_else(|| codegen_err("xulo_closure_func unavailable"))?;
        let env_ptr = self
            .call_runtime("xulo_closure_env", &[checked])?
            .ok_or_else(|| codegen_err("xulo_closure_env unavailable"))?;

        let i64_ty = self.context.i64_type();
        let fn_ptr = cg(self.builder.build_int_to_ptr(
            func_addr.into_int_value(),
            self.context.ptr_type(inkwell::AddressSpace::default()),
            "fnptr",
        ))?;

        let mut call_args: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(9);
        call_args.push(env_ptr.into());
        for arg in args.iter().take(8) {
            let v = self.resolve(arg)?;
            let coerced = self.coerce_to(v, SlotKind::I64)?;
            call_args.push(coerced.into());
        }
        while call_args.len() < 9 {
            call_args.push(BasicValueEnum::from(self.int_const(0)).into());
        }
        let params: Vec<BasicMetadataTypeEnum<'ctx>> = (0..9).map(|_| i64_ty.into()).collect();
        let fn_ty = i64_ty.fn_type(&params, false);
        let call = cg(self
            .builder
            .build_indirect_call(fn_ty, fn_ptr, &call_args, "closure"))?;
        if let (Some(dst), Some(result)) = (dst, call.try_as_basic_value().basic()) {
            self.store(dst, result, StoreMode::Coerce)?;
        }
        Ok(())
    }

    pub(crate) fn lower_extern_call(
        &mut self,
        dst: &Option<LocalId>,
        extern_idx: usize,
        args: &[Operand],
    ) -> Result<(), XuloError> {
        let callee = *self.externs.get(extern_idx).ok_or_else(|| {
            codegen_err(format!("extern function #{extern_idx} was not declared"))
        })?;
        let ext = &self.ir.externs[extern_idx];
        let mut arg_vals: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(args.len());
        for (i, arg) in args.iter().enumerate() {
            let v = self.resolve(arg)?;
            let coerced = match ext.params.get(i) {
                Some(&p) => self.coerce_extern_arg(v, p)?,
                None => v,
            };
            arg_vals.push(coerced.into());
        }
        let call = cg(self.builder.build_call(callee, &arg_vals, "extern"))?;
        if let (Some(dst), Some(result)) = (dst, call.try_as_basic_value().basic()) {
            let coerced = match ext.return_type {
                Some(p) => self.coerce_extern_ret(result, p)?,
                None => result,
            };
            self.store(dst, coerced, StoreMode::Coerce)?;
        }
        Ok(())
    }
}
