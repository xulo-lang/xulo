//! 单条 IR 指令到 LLVM 指令的分派。

use inkwell::values::BasicValueEnum;
use xulo_core::ir::Instruction;

use super::numeric::{BitOp, LogicOp};

use super::*;

impl<'a, 'ctx> Emitter<'a, 'ctx> {
    pub(crate) fn lower_instruction(&mut self, instr: &Instruction) -> Result<(), XuloError> {
        use Instruction::*;
        match instr {
            Label(label) => {
                if let Some(current) = self.builder.get_insert_block()
                    && current.get_terminator().is_none()
                    && let Some(&target) = self.label_blocks.get(label)
                {
                    cg(self.builder.build_unconditional_branch(target))?;
                }
                if let Some(&target) = self.label_blocks.get(label) {
                    self.builder.position_at_end(target);
                }
            }
            Const { dst, value } => self.lower_const(dst, value)?,
            Add { dst, left, right } => {
                self.lower_numeric_binop(dst, left, right, NumericOp::Add)?
            }
            Sub { dst, left, right } => {
                self.lower_numeric_binop(dst, left, right, NumericOp::Sub)?
            }
            Mul { dst, left, right } => {
                self.lower_numeric_binop(dst, left, right, NumericOp::Mul)?
            }
            Div { dst, left, right } => {
                self.lower_numeric_binop(dst, left, right, NumericOp::Div)?
            }
            Mod { dst, left, right } => self.lower_mod(dst, left, right)?,
            Pow { dst, left, right } => self.lower_pow(dst, left, right)?,
            Neg { dst, operand } => self.lower_neg(dst, operand)?,
            BitAnd { dst, left, right } => self.lower_bit_binop(dst, left, right, BitOp::And)?,
            BitOr { dst, left, right } => self.lower_bit_binop(dst, left, right, BitOp::Or)?,
            Xor { dst, left, right } => self.lower_bit_binop(dst, left, right, BitOp::Xor)?,
            Shl { dst, left, right } => self.lower_shift(dst, left, right, false)?,
            Shr { dst, left, right } => self.lower_shift(dst, left, right, true)?,
            BitNot { dst, operand } => self.lower_bit_not(dst, operand)?,
            Eq { dst, left, right } => self.lower_compare(dst, left, right, CompareOp::Eq)?,
            Neq { dst, left, right } => self.lower_compare(dst, left, right, CompareOp::Ne)?,
            Lt { dst, left, right } => self.lower_compare(dst, left, right, CompareOp::Lt)?,
            Gt { dst, left, right } => self.lower_compare(dst, left, right, CompareOp::Gt)?,
            Lte { dst, left, right } => self.lower_compare(dst, left, right, CompareOp::Lte)?,
            Gte { dst, left, right } => self.lower_compare(dst, left, right, CompareOp::Gte)?,
            And { dst, left, right } => self.lower_logic_binop(dst, left, right, LogicOp::And)?,
            Or { dst, left, right } => self.lower_logic_binop(dst, left, right, LogicOp::Or)?,
            Not { dst, operand } => self.lower_not(dst, operand)?,
            Branch {
                condition,
                then_label,
                else_label,
            } => {
                let cond = self.resolve_i64(condition)?;
                let zero = self.int_const(0);
                let cond_bool =
                    cg(self
                        .builder
                        .build_int_compare(IntPredicate::NE, cond, zero, "cond"))?;
                let then_bb = self.label_block(then_label)?;
                let else_bb = self.label_block(else_label)?;
                cg(self
                    .builder
                    .build_conditional_branch(cond_bool, then_bb, else_bb))?;
            }
            Jump(label) => {
                let target = self.label_block(label)?;
                cg(self.builder.build_unconditional_branch(target))?;
            }
            Return(value) => {
                if self.cur_void {
                    cg(self.builder.build_return(None))?;
                } else if let Some(v) = value {
                    let resolved = self.resolve(v)?;
                    let ret = self.coerce_to(resolved, SlotKind::I64)?;
                    cg(self.builder.build_return(Some(&ret)))?;
                } else {
                    let zero = self.int_const(0);
                    let zero: BasicValueEnum<'ctx> = zero.into();
                    cg(self.builder.build_return(Some(&zero)))?;
                }
            }
            SetLocal { dst, src } => self.lower_set_local(dst, src)?,
            GetLocal { dst, src } => self.lower_get_local(dst, src)?,
            RuntimeCall { dst, func, args } => self.lower_runtime_call(dst, func, args)?,
            Call { dst, func, args } => self.lower_call(dst, *func, args)?,
            NewObject { dst, fields } => self.lower_new_object(dst, fields)?,
            NewArray { dst, elements } => self.lower_new_array(dst, elements)?,
            GetField { dst, object, field } => self.lower_get_field(dst, object, field)?,
            SetField {
                object,
                field,
                value,
                tag,
            } => self.lower_set_field(object, field, value, *tag)?,
            GetIndex { dst, array, index } => self.lower_get_index(dst, array, index)?,
            SetIndex {
                array,
                index,
                value,
                tag,
            } => self.lower_set_index(array, index, value, *tag)?,
            FuncAddr { dst, func } => self.lower_func_addr(dst, *func)?,
            MakeClosure { dst, func, env } => self.lower_make_closure(dst, *func, env)?,
            CallIndirect { dst, callee, args } => self.lower_call_indirect(dst, callee, args)?,
            ExternCall {
                dst,
                extern_idx,
                args,
            } => self.lower_extern_call(dst, *extern_idx, args)?,
            Nop => {}
        }
        Ok(())
    }
}
