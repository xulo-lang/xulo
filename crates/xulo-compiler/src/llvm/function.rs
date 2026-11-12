//! 函数体 lowering：入口/参数/标签、指令循环与收尾。

use super::*;

impl<'a, 'ctx> Emitter<'a, 'ctx> {
    pub(crate) fn lower_function(
        &mut self,
        func: &IrFunction,
        func_idx: usize,
        is_entry: bool,
    ) -> Result<(), XuloError> {
        let fv = self.funcs[func_idx];
        self.cur_fn = Some(fv);
        self.cur_void = !func.closure_entry && func.return_type.is_void();
        self.local_slots.clear();
        self.label_blocks.clear();

        let entry_block = self.context.append_basic_block(fv, "entry");
        self.builder.position_at_end(entry_block);

        for (local_id, ty) in &func.locals {
            let kind = SlotKind::of(ty);
            let ty_llvm = abi::ir_slot_type(self.context, ty);
            let ptr = cg(self.builder.build_alloca(ty_llvm, "local"))?;
            self.local_slots.insert(*local_id, Slot { ptr, kind });
        }

        for (i, (local_id, _ty)) in func.params.iter().enumerate() {
            let ptr = cg(self.builder.build_alloca(self.context.i64_type(), "param"))?;
            let arg = fv
                .get_nth_param(i as u32)
                .ok_or_else(|| codegen_err("missing function parameter"))?;
            cg(self.builder.build_store(ptr, arg))?;
            self.local_slots.insert(
                *local_id,
                Slot {
                    ptr,
                    kind: SlotKind::I64,
                },
            );
        }

        if is_entry {
            let strings: Vec<String> = self.ir.strings.clone();
            for s in &strings {
                let ptr = self.string_ptr(s)?;
                let as_int =
                    cg(self
                        .builder
                        .build_ptr_to_int(ptr, self.context.i64_type(), "str"))?;
                self.call_runtime("xulo_register_string", &[as_int.into()])?;
            }
        }

        for label in &func.labels {
            let block = self
                .context
                .append_basic_block(fv, &format!("L{}", label.0));
            self.label_blocks.insert(*label, block);
        }

        for instr in &func.instructions {
            self.lower_instruction(instr)?;
        }

        // 最后一个 block（当前 block）补终止指令。
        if let Some(current) = self.builder.get_insert_block()
            && current.get_terminator().is_none()
        {
            self.emit_fallthrough_return()?;
        }

        // entry 可能因为 switch 到 label 而没有终止指令；补跳转到首个 label。
        if entry_block.get_terminator().is_none() {
            self.builder.position_at_end(entry_block);
            if let Some(first) = func.labels.first()
                && let Some(&target) = self.label_blocks.get(first)
            {
                cg(self.builder.build_unconditional_branch(target))?;
            } else {
                self.emit_fallthrough_return()?;
            }
        }

        self.cur_fn = None;
        Ok(())
    }

    pub(crate) fn emit_fallthrough_return(&self) -> Result<(), XuloError> {
        if self.cur_void {
            cg(self.builder.build_return(None))?;
        } else {
            let zero: BasicValueEnum<'ctx> = self.int_const(0).into();
            cg(self.builder.build_return(Some(&zero)))?;
        }
        Ok(())
    }
}
