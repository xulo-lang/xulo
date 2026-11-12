//! 对象/数组等聚合值的 lowering。

use super::*;

impl<'a, 'ctx> Emitter<'a, 'ctx> {
    pub(crate) fn lower_new_object(
        &mut self,
        dst: &LocalId,
        fields: &[(String, Operand, i64)],
    ) -> Result<(), XuloError> {
        let zero = self.int_const(0);
        let result = self
            .call_runtime("xulo_alloc_object", &[zero.into()])?
            .ok_or_else(|| codegen_err("xulo_alloc_object unavailable"))?;
        self.store(dst, result, StoreMode::Coerce)?;
        for (field_name, field_val, tag) in fields {
            let obj = self.load_local(dst)?;
            let raw = self.resolve(field_val)?;
            let val = self.coerce_to(raw, SlotKind::I64)?;
            let tag_val = self.int_const(*tag);
            let field_ptr = self.string_ptr(field_name)?;
            let field_int =
                cg(self
                    .builder
                    .build_ptr_to_int(field_ptr, self.context.i64_type(), "f"))?;
            self.call_runtime(
                "xulo_object_set",
                &[obj, field_int.into(), val, tag_val.into()],
            )?;
        }
        Ok(())
    }

    pub(crate) fn lower_new_array(
        &mut self,
        dst: &LocalId,
        elements: &[(Operand, i64)],
    ) -> Result<(), XuloError> {
        let zero = self.int_const(0);
        let result = self
            .call_runtime("xulo_alloc_array", &[zero.into()])?
            .ok_or_else(|| codegen_err("xulo_alloc_array unavailable"))?;
        self.store(dst, result, StoreMode::Coerce)?;
        for (elem, tag) in elements {
            let arr = self.load_local(dst)?;
            let raw = self.resolve(elem)?;
            let val = self.coerce_to(raw, SlotKind::I64)?;
            let tag_val = self.int_const(*tag);
            self.call_runtime("xulo_array_push", &[arr, val, tag_val.into()])?;
        }
        Ok(())
    }

    pub(crate) fn lower_get_field(
        &mut self,
        dst: &LocalId,
        object: &Operand,
        field: &str,
    ) -> Result<(), XuloError> {
        let obj = self.resolve(object)?;
        let field_ptr = self.string_ptr(field)?;
        let field_int = cg(self
            .builder
            .build_ptr_to_int(field_ptr, self.context.i64_type(), "f"))?;
        let result = self
            .call_runtime("xulo_object_get", &[obj, field_int.into()])?
            .ok_or_else(|| codegen_err("xulo_object_get unavailable"))?;
        self.store(dst, result, StoreMode::Coerce)
    }

    pub(crate) fn lower_set_field(
        &mut self,
        object: &Operand,
        field: &str,
        value: &Operand,
        tag: i64,
    ) -> Result<(), XuloError> {
        let obj = self.resolve(object)?;
        let raw = self.resolve(value)?;
        let val = self.coerce_to(raw, SlotKind::I64)?;
        let tag_val = self.int_const(tag);
        let field_ptr = self.string_ptr(field)?;
        let field_int = cg(self
            .builder
            .build_ptr_to_int(field_ptr, self.context.i64_type(), "f"))?;
        self.call_runtime(
            "xulo_object_set",
            &[obj, field_int.into(), val, tag_val.into()],
        )?;
        Ok(())
    }

    pub(crate) fn lower_get_index(
        &mut self,
        dst: &LocalId,
        array: &Operand,
        index: &Operand,
    ) -> Result<(), XuloError> {
        let arr = self.resolve(array)?;
        let idx = self.resolve_i64(index)?;
        let result = self
            .call_runtime("xulo_array_get", &[arr, idx.into()])?
            .ok_or_else(|| codegen_err("xulo_array_get unavailable"))?;
        self.store(dst, result, StoreMode::Coerce)
    }

    pub(crate) fn lower_set_index(
        &mut self,
        array: &Operand,
        index: &Operand,
        value: &Operand,
        tag: i64,
    ) -> Result<(), XuloError> {
        let arr = self.resolve(array)?;
        let idx = self.resolve_i64(index)?;
        let raw = self.resolve(value)?;
        let val = self.coerce_to(raw, SlotKind::I64)?;
        let tag_val = self.int_const(tag);
        self.call_runtime("xulo_array_set", &[arr, idx.into(), val, tag_val.into()])?;
        Ok(())
    }
}
