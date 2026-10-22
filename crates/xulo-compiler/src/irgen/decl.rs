use super::*;

impl IrGenerator {
    /// 函数返回值是 Math::sqrt 的 F64 位模式时记下（print 需要 tag=2 还原小数）
    pub(super) fn note_float_bit_returns(&mut self, ir_name: &str, f: &FnDef) {
        if !matches!(f.return_type, Some(Type::Number)) {
            return;
        }
        let is_sqrt = f.body.statements.iter().any(|s| match s {
            Statement::Return(r) => r.value.as_ref().is_some_and(|e| self.expr_is_sqrt_call(e)),
            Statement::Expr(e) => self.expr_is_sqrt_call(&e.expr),
            _ => false,
        });
        if is_sqrt {
            self.float_bit_returns.insert(ir_name.to_string());
        }
    }

    pub(super) fn expr_is_sqrt_call(&self, expr: &Expression) -> bool {
        matches!(expr, Expression::Call(call)
            if call.object.is_none() && call.callee == "std::math::sqrt")
    }

    pub(super) fn register_function(&mut self, f: &FnDef) -> Result<FuncId, XuloError> {
        let name = self.resolve_value(&f.name);
        self.register_named_function(&name, f)
    }

    /// 以指定的 IR 名注册函数（impl 方法用 `Type::method` 作名）
    pub(super) fn register_named_function(
        &mut self,
        name: &str,
        f: &FnDef,
    ) -> Result<FuncId, XuloError> {
        let params: Vec<(LocalId, IrType)> = f
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mut ty = self.type_to_ir(p.type_annotation.as_ref());
                // 形参不允许 Void（self 等无注解参数按槽位处理）
                if matches!(ty, IrType::Void) {
                    ty = IrType::I64;
                }
                (LocalId(i), ty)
            })
            .collect();

        let return_type = f
            .return_type
            .as_ref()
            .map(|t| self.type_to_ir(Some(t)))
            .unwrap_or(IrType::Void);

        // 保存 AST 返回类型注解（用于泛型函数推断）
        self.func_return_types
            .insert(name.to_string(), f.return_type.clone());
        // 形参表：命名参数重排 / 缺省参数在调用点补齐
        self.func_params.insert(
            name.to_string(),
            f.params
                .iter()
                .map(|p| (p.name.clone(), p.default.clone()))
                .collect(),
        );

        let func = IrFunction::new(name.to_string(), params, return_type);
        Ok(self.module.add_function(func))
    }
}
