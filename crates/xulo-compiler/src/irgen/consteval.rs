use super::*;

impl IrGenerator {
    /// 记录模块级 `const`：使用点内联初始化表达式
    pub(super) fn collect_const(&mut self, module_id: &str, b: &LetBinding) {
        if !b.is_const {
            return;
        }
        let Some(expr) = &b.value else { return };
        let key = self.resolve_value(&b.name);
        self.consts.insert(
            key,
            ConstDef {
                module_id: module_id.to_string(),
                tag: Self::const_static_tag(expr),
                ty: Self::const_static_type(expr),
                expr: expr.clone(),
            },
        );
    }

    /// 字面量（含取负）的静态标签
    pub(super) fn const_static_tag(expr: &Expression) -> i64 {
        match expr {
            Expression::Literal { value, .. } => match value {
                Literal::String(_) => 0,
                Literal::Number(n) => {
                    if n.fract() == 0.0 {
                        1
                    } else {
                        2
                    }
                }
                Literal::Boolean(_) => 5,
                Literal::Null => 6,
                Literal::List(_) => 3,
                Literal::Object(_) => 4,
            },
            Expression::Unary(u)
                if matches!(
                    u.operator,
                    UnaryOperator::Neg | UnaryOperator::Move | UnaryOperator::Copy
                ) =>
            {
                Self::const_static_tag(&u.operand)
            }
            _ => -1,
        }
    }

    /// 字面量（含取负）的静态源级类型
    pub(super) fn const_static_type(expr: &Expression) -> Option<Type> {
        match expr {
            Expression::Literal { value, .. } => Some(match value {
                Literal::String(_) => Type::String,
                Literal::Number(_) => Type::Number,
                Literal::Boolean(_) => Type::Boolean,
                Literal::Null => Type::Null,
                Literal::List(_) => Type::List(Box::new(Type::Any)),
                Literal::Object(_) => Type::Object,
            }),
            Expression::Unary(u) if matches!(u.operator, UnaryOperator::Neg) => {
                Self::const_static_type(&u.operand)
            }
            _ => None,
        }
    }

    /// 当前模块里 `name` 指向的模块级常量
    pub(super) fn const_def(&self, name: &str) -> Option<&ConstDef> {
        let key = self.resolve_value(name);
        self.consts.get(&key)
    }

    /// 内联 const 的初始化表达式：切到定义模块的上下文生成（`use` 绑定/
    /// 模块前缀都按定义处解析），并检测自引用。
    pub(super) fn generate_const_ref(
        &mut self,
        name: &str,
        def: &ConstDef,
    ) -> Result<Operand, XuloError> {
        if self.const_stack.iter().any(|n| n == name) {
            return Err(XuloError::new(
                ErrorKind::Semantic,
                format!("cyclic `const` initializer: {name}"),
            ));
        }
        self.const_stack.push(name.to_string());
        let saved = std::mem::replace(&mut self.current_module, def.module_id.clone());
        let out = self.generate_expression(&def.expr);
        self.current_module = saved;
        self.const_stack.pop();
        out
    }
}
