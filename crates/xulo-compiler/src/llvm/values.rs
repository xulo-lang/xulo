//! 值解析、槽位读写、类型转换与字符串/标签辅助。

use super::*;

impl<'a, 'ctx> Emitter<'a, 'ctx> {
    pub(crate) fn resolve(&mut self, operand: &Operand) -> Result<BasicValueEnum<'ctx>, XuloError> {
        match operand {
            Operand::Const(value) => match value {
                xulo_core::ir::IrValue::Bool(b) => {
                    Ok(self.int_const(if *b { 1 } else { 0 }).into())
                }
                xulo_core::ir::IrValue::I64(n) => Ok(self.int_const(*n).into()),
                xulo_core::ir::IrValue::F64(n) => {
                    Ok(self.context.f64_type().const_float(*n).into())
                }
                xulo_core::ir::IrValue::Null => Ok(self.int_const(0).into()),
                xulo_core::ir::IrValue::String(idx) => match self.ir.strings.get(*idx) {
                    Some(s) => {
                        let ptr = self.string_ptr(s)?;
                        Ok(cg(self
                            .builder
                            .build_ptr_to_int(ptr, self.context.i64_type(), "str"))?
                        .into())
                    }
                    None => Ok(self.int_const(0).into()),
                },
            },
            Operand::Local(id) => self.load_local(id),
        }
    }

    pub(crate) fn resolve_i64(&mut self, operand: &Operand) -> Result<IntValue<'ctx>, XuloError> {
        let value = self.resolve(operand)?;
        self.to_i64(value)
    }

    pub(crate) fn load_local(&self, id: &LocalId) -> Result<BasicValueEnum<'ctx>, XuloError> {
        let slot = self
            .local_slots
            .get(id)
            .ok_or_else(|| codegen_err(format!("local {id:?} was not allocated")))?;
        let pointee: BasicTypeEnum<'ctx> = match slot.kind {
            SlotKind::I64 => self.context.i64_type().into(),
            SlotKind::F64 => self.context.f64_type().into(),
        };
        cg(self.builder.build_load(pointee, slot.ptr, "local"))
    }

    pub(crate) fn local_slot_kind(&self, id: &LocalId) -> Option<SlotKind> {
        self.local_slots.get(id).map(|slot| slot.kind)
    }

    pub(crate) fn store(
        &mut self,
        dst: &LocalId,
        value: BasicValueEnum<'ctx>,
        mode: StoreMode,
    ) -> Result<(), XuloError> {
        let slot = *self
            .local_slots
            .get(dst)
            .ok_or_else(|| codegen_err(format!("local {dst:?} was not allocated")))?;
        let adjusted = match mode {
            StoreMode::Coerce => self.coerce_to(value, slot.kind)?,
            StoreMode::Fit => self.fit_to(value, slot.kind)?,
        };
        cg(self.builder.build_store(slot.ptr, adjusted))?;
        Ok(())
    }

    pub(crate) fn int_const(&self, value: i64) -> IntValue<'ctx> {
        self.context.i64_type().const_int(value as u64, true)
    }

    pub(crate) fn is_f64(&self, value: BasicValueEnum<'ctx>) -> bool {
        value.is_float_value() && value.into_float_value().get_type() == self.context.f64_type()
    }

    pub(crate) fn is_f32(&self, value: BasicValueEnum<'ctx>) -> bool {
        value.is_float_value() && value.into_float_value().get_type() == self.context.f32_type()
    }

    /// 整数域操作数：f64/f32 按解释器 `as i64` 的饱和语义截断。
    pub(crate) fn to_i64(&self, value: BasicValueEnum<'ctx>) -> Result<IntValue<'ctx>, XuloError> {
        if value.is_int_value() {
            return Ok(value.into_int_value());
        }
        let f = value.into_float_value();
        let call = cg(self
            .builder
            .build_call(self.fptosi_sat, &[f.into()], "fptosi"))?;
        let result = call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| codegen_err("llvm.fptosi.sat produced no value"))?;
        Ok(result.into_int_value())
    }

    pub(crate) fn to_f64(
        &self,
        value: BasicValueEnum<'ctx>,
    ) -> Result<FloatValue<'ctx>, XuloError> {
        if value.is_float_value() {
            let f = value.into_float_value();
            if f.get_type() == self.context.f64_type() {
                return Ok(f);
            }
            return cg(self
                .builder
                .build_float_ext(f, self.context.f64_type(), "f64"));
        }
        let i = value.into_int_value();
        cg(self
            .builder
            .build_signed_int_to_float(i, self.context.f64_type(), "f64"))
    }

    /// `coerce_to`：I64/F64 槽位间按 Cranelift 后端语义对齐（f64 -> i64 走
    /// 位模式 bitcast，i64 -> f64 走数值转换）。
    pub(crate) fn coerce_to(
        &self,
        value: BasicValueEnum<'ctx>,
        want: SlotKind,
    ) -> Result<BasicValueEnum<'ctx>, XuloError> {
        match want {
            SlotKind::I64 => {
                if value.is_int_value() {
                    Ok(value)
                } else if self.is_f64(value) || self.is_f32(value) {
                    cg(self
                        .builder
                        .build_bitcast(value, self.context.i64_type(), "bits"))
                } else {
                    Ok(value)
                }
            }
            SlotKind::F64 => {
                if self.is_f64(value) {
                    Ok(value)
                } else if value.is_int_value() {
                    Ok(cg(self.builder.build_signed_int_to_float(
                        value.into_int_value(),
                        self.context.f64_type(),
                        "num",
                    ))?
                    .into())
                } else if self.is_f32(value) {
                    Ok(cg(self.builder.build_float_ext(
                        value.into_float_value(),
                        self.context.f64_type(),
                        "f64",
                    ))?
                    .into())
                } else {
                    Ok(value)
                }
            }
        }
    }

    /// `fit_numeric_slot`：整数域结果放回槽位，f64 -> i64 走饱和截断。
    pub(crate) fn fit_to(
        &self,
        value: BasicValueEnum<'ctx>,
        want: SlotKind,
    ) -> Result<BasicValueEnum<'ctx>, XuloError> {
        match want {
            SlotKind::I64 => {
                if value.is_float_value() {
                    Ok(self.to_i64(value)?.into())
                } else {
                    Ok(value)
                }
            }
            SlotKind::F64 => self.coerce_to(value, SlotKind::F64),
        }
    }

    pub(crate) fn string_ptr(&mut self, s: &str) -> Result<PointerValue<'ctx>, XuloError> {
        if let Some(ptr) = self.strings.get(s) {
            return Ok(*ptr);
        }
        let name = format!("str_{}", self.next_string);
        self.next_string += 1;
        let global = cg(self.builder.build_global_string_ptr(s, &name))?;
        global.set_linkage(Linkage::Private);
        global.set_unnamed_addr(true);
        let ptr = global.as_pointer_value();
        self.strings.insert(s.to_string(), ptr);
        Ok(ptr)
    }

    pub(crate) fn label_block(&self, label: &Label) -> Result<BasicBlock<'ctx>, XuloError> {
        self.label_blocks
            .get(label)
            .copied()
            .ok_or_else(|| codegen_err(format!("label {label:?} was not declared")))
    }

    pub(crate) fn lower_const(
        &mut self,
        dst: &LocalId,
        value: &xulo_core::ir::IrValue,
    ) -> Result<(), XuloError> {
        let val = match value {
            xulo_core::ir::IrValue::Bool(b) => self.int_const(if *b { 1 } else { 0 }).into(),
            xulo_core::ir::IrValue::I64(n) => self.int_const(*n).into(),
            xulo_core::ir::IrValue::F64(n) => {
                let bits = self.int_const(n.to_bits() as i64);
                self.call_runtime("xulo_register_float", &[bits.into()])?;
                self.context.f64_type().const_float(*n).into()
            }
            xulo_core::ir::IrValue::Null => self.int_const(0).into(),
            xulo_core::ir::IrValue::String(idx) => match self.ir.strings.get(*idx) {
                Some(s) => {
                    let ptr = self.string_ptr(s)?;
                    cg(self
                        .builder
                        .build_ptr_to_int(ptr, self.context.i64_type(), "str"))?
                    .into()
                }
                None => self.int_const(0).into(),
            },
        };
        self.store(dst, val, StoreMode::Coerce)
    }

    pub(crate) fn lower_set_local(
        &mut self,
        dst: &LocalId,
        src: &Operand,
    ) -> Result<(), XuloError> {
        let val = self.resolve(src)?;
        self.store(dst, val, StoreMode::Coerce)
    }

    pub(crate) fn lower_get_local(
        &mut self,
        dst: &LocalId,
        src: &LocalId,
    ) -> Result<(), XuloError> {
        let val = self.load_local(src)?;
        self.store(dst, val, StoreMode::Coerce)
    }
}
