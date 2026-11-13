//! 数值、比较、位运算与 trap 语义。

use super::*;

impl<'a, 'ctx> Emitter<'a, 'ctx> {
    pub(crate) fn numeric_binop(
        &mut self,
        left: &Operand,
        right: &Operand,
        op: NumericOp,
    ) -> Result<BasicValueEnum<'ctx>, XuloError> {
        let l = self.resolve(left)?;
        let r = self.resolve(right)?;
        if self.is_f64(l) || self.is_f64(r) {
            let lf = self.to_f64(l)?;
            let rf = self.to_f64(r)?;
            let val = match op {
                NumericOp::Add => cg(self.builder.build_float_add(lf, rf, "add")),
                NumericOp::Sub => cg(self.builder.build_float_sub(lf, rf, "sub")),
                NumericOp::Mul => cg(self.builder.build_float_mul(lf, rf, "mul")),
                NumericOp::Div => cg(self.builder.build_float_div(lf, rf, "div")),
            }?;
            Ok(val.into())
        } else {
            let li = self.to_i64(l)?;
            let ri = self.to_i64(r)?;
            let val = match op {
                NumericOp::Add => cg(self.builder.build_int_add(li, ri, "add"))?,
                NumericOp::Sub => cg(self.builder.build_int_sub(li, ri, "sub"))?,
                NumericOp::Mul => cg(self.builder.build_int_mul(li, ri, "mul"))?,
                NumericOp::Div => {
                    self.emit_div_guard(li, ri, true)?;
                    cg(self.builder.build_int_signed_div(li, ri, "div"))?
                }
            };
            Ok(val.into())
        }
    }

    pub(crate) fn compare(
        &mut self,
        left: &Operand,
        right: &Operand,
        op: CompareOp,
    ) -> Result<BasicValueEnum<'ctx>, XuloError> {
        let l = self.resolve(left)?;
        let r = self.resolve(right)?;
        let cmp = if self.is_f64(l) || self.is_f64(r) {
            let lf = self.to_f64(l)?;
            let rf = self.to_f64(r)?;
            let pred = match op {
                CompareOp::Eq => FloatPredicate::OEQ,
                CompareOp::Ne => FloatPredicate::ONE,
                CompareOp::Lt => FloatPredicate::OLT,
                CompareOp::Gt => FloatPredicate::OGT,
                CompareOp::Lte => FloatPredicate::OLE,
                CompareOp::Gte => FloatPredicate::OGE,
            };
            cg(self.builder.build_float_compare(pred, lf, rf, "cmp"))?
        } else {
            let li = self.to_i64(l)?;
            let ri = self.to_i64(r)?;
            let pred = match op {
                CompareOp::Eq => IntPredicate::EQ,
                CompareOp::Ne => IntPredicate::NE,
                CompareOp::Lt => IntPredicate::SLT,
                CompareOp::Gt => IntPredicate::SGT,
                CompareOp::Lte => IntPredicate::SLE,
                CompareOp::Gte => IntPredicate::SGE,
            };
            cg(self.builder.build_int_compare(pred, li, ri, "cmp"))?
        };
        let widened = cg(self
            .builder
            .build_int_z_extend(cmp, self.context.i64_type(), "cmp.i64"))?;
        Ok(widened.into())
    }

    /// 整数除法/取余的运行时保护：与 Cranelift 的 trap 行为对齐（除零 /
    /// `INT_MIN / -1` 溢出直接 trap）。LLVM 对这两种情况是 UB，不能交给优化器。
    pub(crate) fn emit_div_guard(
        &mut self,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
        check_overflow: bool,
    ) -> Result<(), XuloError> {
        let i64_ty = self.context.i64_type();
        let zero = i64_ty.const_zero();
        let is_zero = cg(self
            .builder
            .build_int_compare(IntPredicate::EQ, rhs, zero, "divzero"))?;
        let mut bad = is_zero;
        if check_overflow {
            let min = i64_ty.const_int(i64::MIN as u64, true);
            let minus_one = i64_ty.const_int((-1i64) as u64, true);
            let lhs_min = cg(self
                .builder
                .build_int_compare(IntPredicate::EQ, lhs, min, "dmin"))?;
            let rhs_m1 =
                cg(self
                    .builder
                    .build_int_compare(IntPredicate::EQ, rhs, minus_one, "dm1"))?;
            let both = cg(self.builder.build_and(lhs_min, rhs_m1, "dovf"))?;
            bad = cg(self.builder.build_or(bad, both, "dbad"))?;
        }

        self.trap_on(bad)
    }

    /// `bad` 为真时跳到 trap 块并终止控制流，否则继续。
    pub(crate) fn trap_on(&mut self, bad: IntValue<'ctx>) -> Result<(), XuloError> {
        let fv = self
            .cur_fn
            .ok_or_else(|| codegen_err("trap emitted outside a function"))?;
        let trap_bb = self.context.append_basic_block(fv, "trap");
        let cont_bb = self.context.append_basic_block(fv, "trap.cont");
        cg(self.builder.build_conditional_branch(bad, trap_bb, cont_bb))?;
        self.builder.position_at_end(trap_bb);
        cg(self.builder.build_call(self.trap_fn, &[], ""))?;
        cg(self.builder.build_unreachable())?;
        self.builder.position_at_end(cont_bb);
        Ok(())
    }

    /// f64 -> i64 的 trap 语义（Cranelift `fcvt_to_sint`）：NaN 与越界
    /// 直接 trap，范围内的结果与 LLVM `fptosi` 一致。
    pub(crate) fn float_to_i64_trapping(
        &mut self,
        f: FloatValue<'ctx>,
    ) -> Result<IntValue<'ctx>, XuloError> {
        let fty = self.context.f64_type();
        let min = fty.const_float(-9_223_372_036_854_775_808.0);
        let max = fty.const_float(9_223_372_036_854_775_808.0);
        let is_nan = cg(self
            .builder
            .build_float_compare(FloatPredicate::UNO, f, f, "f2i.nan"))?;
        let too_low = cg(self
            .builder
            .build_float_compare(FloatPredicate::OLT, f, min, "f2i.low"))?;
        let too_high =
            cg(self
                .builder
                .build_float_compare(FloatPredicate::OGE, f, max, "f2i.high"))?;
        let bad = cg(self.builder.build_or(is_nan, too_low, "f2i.bad"))?;
        let bad = cg(self.builder.build_or(bad, too_high, "f2i.bad"))?;
        self.trap_on(bad)?;
        cg(self
            .builder
            .build_float_to_signed_int(f, self.context.i64_type(), "f2i"))
    }

    pub(crate) fn float_trunc(
        &self,
        value: FloatValue<'ctx>,
    ) -> Result<FloatValue<'ctx>, XuloError> {
        let call = cg(self
            .builder
            .build_call(self.trunc_f64, &[value.into()], "trunc"))?;
        let result = call
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| codegen_err("llvm.trunc produced no value"))?;
        Ok(result.into_float_value())
    }

    pub(crate) fn lower_numeric_binop(
        &mut self,
        dst: &LocalId,
        left: &Operand,
        right: &Operand,
        op: NumericOp,
    ) -> Result<(), XuloError> {
        let val = self.numeric_binop(left, right, op)?;
        self.store(dst, val, StoreMode::Coerce)
    }

    pub(crate) fn lower_mod(
        &mut self,
        dst: &LocalId,
        left: &Operand,
        right: &Operand,
    ) -> Result<(), XuloError> {
        let l = self.resolve(left)?;
        let r = self.resolve(right)?;
        let val = if self.is_f64(l) || self.is_f64(r) {
            let lf = self.to_f64(l)?;
            let rf = self.to_f64(r)?;
            let q = cg(self.builder.build_float_div(lf, rf, "mod.q"))?;
            let t = self.float_trunc(q)?;
            let p = cg(self.builder.build_float_mul(t, rf, "mod.p"))?;
            cg(self.builder.build_float_sub(lf, p, "mod"))?.into()
        } else {
            let li = self.to_i64(l)?;
            let ri = self.to_i64(r)?;
            self.emit_div_guard(li, ri, false)?;
            let minus_one = self.int_const(-1);
            let is_m1 =
                cg(self
                    .builder
                    .build_int_compare(IntPredicate::EQ, ri, minus_one, "mod.isneg1"))?;
            let safe = cg(self
                .builder
                .build_select(is_m1, self.int_const(1), ri, "mod.safe"))?;
            cg(self
                .builder
                .build_int_signed_rem(li, safe.into_int_value(), "mod"))?
            .into()
        };
        self.store(dst, val, StoreMode::Fit)
    }

    pub(crate) fn lower_pow(
        &mut self,
        dst: &LocalId,
        left: &Operand,
        right: &Operand,
    ) -> Result<(), XuloError> {
        let l = self.resolve(left)?;
        let r = self.resolve(right)?;
        let lf = self.to_f64(l)?;
        let rf = self.to_f64(r)?;
        if let Some(result) = self.call_runtime("xulo_math_pow", &[lf.into(), rf.into()])? {
            let kind = self.local_slot_kind(dst);
            let val = match kind {
                Some(SlotKind::F64) => result,
                Some(SlotKind::I64) => {
                    let f = result.into_float_value();
                    self.float_to_i64_trapping(f)?.into()
                }
                None => result,
            };
            self.store(dst, val, StoreMode::Coerce)?;
        }
        Ok(())
    }

    pub(crate) fn lower_neg(&mut self, dst: &LocalId, operand: &Operand) -> Result<(), XuloError> {
        let o = self.resolve(operand)?;
        let val = if self.is_f64(o) {
            let f = o.into_float_value();
            cg(self.builder.build_float_neg(f, "neg"))?.into()
        } else {
            let i = self.to_i64(o)?;
            cg(self.builder.build_int_neg(i, "neg"))?.into()
        };
        self.store(dst, val, StoreMode::Coerce)
    }

    pub(crate) fn lower_bit_binop(
        &mut self,
        dst: &LocalId,
        left: &Operand,
        right: &Operand,
        op: BitOp,
    ) -> Result<(), XuloError> {
        let l = self.resolve_i64(left)?;
        let r = self.resolve_i64(right)?;
        let val = match op {
            BitOp::And => cg(self.builder.build_and(l, r, "and"))?,
            BitOp::Or => cg(self.builder.build_or(l, r, "or"))?,
            BitOp::Xor => cg(self.builder.build_xor(l, r, "xor"))?,
        };
        self.store(dst, val.into(), StoreMode::Fit)
    }

    pub(crate) fn lower_shift(
        &mut self,
        dst: &LocalId,
        left: &Operand,
        right: &Operand,
        right_shift: bool,
    ) -> Result<(), XuloError> {
        let l = self.resolve_i64(left)?;
        let r = self.resolve_i64(right)?;
        let mode = self.int_const(if right_shift { 1 } else { 0 });
        let val = self
            .call_runtime("xulo_shift", &[l.into(), r.into(), mode.into()])?
            .ok_or_else(|| codegen_err("xulo_shift unavailable"))?;
        self.store(dst, val, StoreMode::Fit)
    }

    pub(crate) fn lower_bit_not(
        &mut self,
        dst: &LocalId,
        operand: &Operand,
    ) -> Result<(), XuloError> {
        let o = self.resolve_i64(operand)?;
        let val = cg(self.builder.build_not(o, "not"))?;
        self.store(dst, val.into(), StoreMode::Fit)
    }

    pub(crate) fn lower_logic_binop(
        &mut self,
        dst: &LocalId,
        left: &Operand,
        right: &Operand,
        op: LogicOp,
    ) -> Result<(), XuloError> {
        let l = self.resolve_i64(left)?;
        let r = self.resolve_i64(right)?;
        let val = match op {
            LogicOp::And => cg(self.builder.build_and(l, r, "and"))?,
            LogicOp::Or => cg(self.builder.build_or(l, r, "or"))?,
        };
        self.store(dst, val.into(), StoreMode::Coerce)
    }

    pub(crate) fn lower_not(&mut self, dst: &LocalId, operand: &Operand) -> Result<(), XuloError> {
        let o = self.resolve_i64(operand)?;
        let zero = self.int_const(0);
        let is_zero = cg(self
            .builder
            .build_int_compare(IntPredicate::EQ, o, zero, "not.eq"))?;
        let val =
            cg(self
                .builder
                .build_select(is_zero, self.int_const(1), self.int_const(0), "not"))?;
        self.store(dst, val, StoreMode::Coerce)
    }

    pub(crate) fn lower_compare(
        &mut self,
        dst: &LocalId,
        left: &Operand,
        right: &Operand,
        op: CompareOp,
    ) -> Result<(), XuloError> {
        let v = self.compare(left, right, op)?;
        self.store(dst, v, StoreMode::Coerce)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum BitOp {
    And,
    Or,
    Xor,
}

#[derive(Clone, Copy)]
pub(crate) enum LogicOp {
    And,
    Or,
}
