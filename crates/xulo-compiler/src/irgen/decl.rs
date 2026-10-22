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

    /// extern 块收集：每条 item 落成 [`IrExternFn`]（C 符号 + SysV 边界类），
    /// 本地名（`as` 别名或声明名）登记进 `extern_map`；形参表一并注册，
    /// 命名实参可按形参序重排（extern 参数没有缺省值）。
    pub(super) fn collect_externs(&mut self, b: &ExternBlockDecl) {
        for item in &b.items {
            let params = item
                .params
                .iter()
                .map(|p| Self::abi_param_of(p.type_annotation.as_ref()))
                .collect();
            let idx = self.module.externs.len();
            self.module.externs.push(IrExternFn {
                symbol: item.name.clone(),
                params,
                return_type: Self::abi_return_of(item.return_type.as_ref()),
                abi: b.abi.clone(),
            });
            let key = self.resolve_value(&item.local_name);
            // 返回类型注解同样登记：print 的 tag 推断与泛型推断按声明类型走
            self.func_return_types
                .insert(key.clone(), item.return_type.clone());
            self.func_params.insert(
                key.clone(),
                item.params
                    .iter()
                    .map(|p| (p.name.clone(), p.default.clone()))
                    .collect(),
            );
            self.extern_map.insert(key, idx);
        }
    }

    /// 边界类型 -> C ABI 类（spec ffi.md 的 ABI-safe 表）。语义层 E0901 只放行
    /// 这些拼写：整宽按符号性逐级映射（xulo 内部整数槽是 I64，窄类只在边界
    /// 上按位截断/扩展）；`usize`/`isize` 是字长整类；`Ptr`/`Ptr?` 与 `_Bool`
    /// 之外的 `bool` 都是 64 位/1 字节整类；未知或缺省注解按 U64 兜底。
    fn abi_param_of(ty: Option<&Type>) -> IrAbiParam {
        let Some(t) = ty else {
            return IrAbiParam::U64;
        };
        match t {
            Type::Boolean => IrAbiParam::Bool,
            // E0901 只放行 `Ptr?`；可空整数/布尔不进边界
            Type::Optional(inner) => Self::abi_param_of(Some(inner)),
            Type::Named(n, _) => match n.as_str() {
                "i8" => IrAbiParam::S8,
                "i16" => IrAbiParam::S16,
                "i32" => IrAbiParam::S32,
                "i64" => IrAbiParam::S64,
                "u8" => IrAbiParam::U8,
                "u16" => IrAbiParam::U16,
                "u32" => IrAbiParam::U32,
                "u64" => IrAbiParam::U64,
                "usize" => IrAbiParam::U64,
                "isize" => IrAbiParam::S64,
                "f32" => IrAbiParam::F32,
                "f64" => IrAbiParam::F64,
                "Ptr" => IrAbiParam::U64,
                _ => IrAbiParam::U64,
            },
            _ => IrAbiParam::U64,
        }
    }

    /// 返回注解 -> C 返回类：无注解或 `unit` 是 void，其余按边界类映射。
    fn abi_return_of(ty: Option<&Type>) -> Option<IrAbiParam> {
        match ty {
            None => None,
            Some(Type::Named(n, _)) if n == "unit" => None,
            Some(t) => Some(Self::abi_param_of(Some(t))),
        }
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
