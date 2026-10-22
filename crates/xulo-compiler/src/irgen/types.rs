use super::*;

impl IrGenerator {
    /// 记录 struct 定义（键为规范化类型名，结构体构造按字段顺序/默认值）
    pub(super) fn collect_struct(&mut self, def: &StructDef) {
        let key = self.resolve_type(&def.name);
        self.structs.insert(key, def.fields.clone());
    }

    /// 记录 enum 定义（键为规范化类型名，变体序号即构造/match 的 tag）
    pub(super) fn collect_enum(&mut self, def: &EnumDef) {
        let key = self.resolve_type(&def.name);
        self.enums
            .insert(key, def.variants.iter().map(|v| v.name.clone()).collect());
    }

    /// 枚举/类型引用的展示名（运行时 `Enum::Variant` 用声明名，别名导入
    /// 不应把本地别名写进输出）
    pub(super) fn enum_display_name(&self, name: &str) -> String {
        if let Some(ctx) = self.ctx()
            && let Some((_, member)) = ctx.aliases.get(name)
        {
            return member.clone();
        }
        name.to_string()
    }

    /// `Enum::Variant` 的变体序号（声明序）
    pub(super) fn enum_variant_index(&self, enum_name: &str, variant: &str) -> Option<usize> {
        let key = self.resolve_type(enum_name);
        self.enums
            .get(&key)
            .and_then(|vs| vs.iter().position(|v| v == variant))
    }

    /// 从表达式取结构体（Named）类型名
    pub(super) fn struct_name_of_expr(&self, expr: &Expression) -> Option<String> {
        match self.infer_source_type_from_expr(expr) {
            // 类型名的规范化（模块前缀/导入别名）在查表处统一进行
            Some(Type::Named(name, _)) => Some(self.resolve_type(&name)),
            _ => None,
        }
    }

    /// 枚举构造：AllocEnum + 逐 payload 槽 EnumSet（值 + 静态推断的标签）
    pub(super) fn generate_enum_construct(
        &mut self,
        enum_name: &str,
        variant: &str,
        args: &[CallArg],
    ) -> Result<Operand, XuloError> {
        let Some(variant_idx) = self.enum_variant_index(enum_name, variant) else {
            return Err(XuloError::new(
                ErrorKind::Semantic,
                format!("undefined variant: {}::{}", enum_name, variant),
            ));
        };
        let func_idx = self.current_func.unwrap();
        let type_str_local = self.module.functions[func_idx].add_local(IrType::I64);
        // 运行时展示用声明名（别名导入不把本地别名写进 `E::V` 输出）
        let type_idx = self.module.add_string(self.enum_display_name(enum_name));
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Const {
                dst: type_str_local,
                value: IrValue::String(type_idx),
            });
        let variant_str_local = self.module.functions[func_idx].add_local(IrType::I64);
        let variant_str_idx = self.module.add_string(variant.to_string());
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Const {
                dst: variant_str_local,
                value: IrValue::String(variant_str_idx),
            });

        let dst = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(dst),
                func: RuntimeFn::AllocEnum,
                args: vec![
                    Operand::Local(type_str_local),
                    Operand::Local(variant_str_local),
                    Operand::Const(IrValue::I64(variant_idx as i64)),
                    Operand::Const(IrValue::I64(args.len() as i64)),
                ],
            });
        for (i, arg) in args.iter().enumerate() {
            let value = self.generate_expression(&arg.value)?;
            let tag = self.determine_element_tag(&arg.value);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: None,
                    func: RuntimeFn::EnumSet,
                    args: vec![
                        Operand::Local(dst),
                        Operand::Const(IrValue::I64(i as i64)),
                        value,
                        Operand::Const(IrValue::I64(tag)),
                    ],
                });
        }
        Ok(Operand::Local(dst))
    }

    /// 结构体构造：按字段顺序 NewObject；命名参数、位置参数、默认值、缺省 null
    pub(super) fn generate_struct_ctor(
        &mut self,
        call: &Call,
        fields: &[StructField],
    ) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let mut used = vec![false; call.arguments.len()];
        let mut field_ops: Vec<(String, Operand, i64)> = Vec::new();

        for field in fields {
            // 命名参数优先，其次按位置顺序；都没有则用默认值，最后退化到 null
            let named = call
                .arguments
                .iter()
                .enumerate()
                .position(|(i, a)| !used[i] && a.name.as_deref() == Some(field.name.as_str()));
            let positional = call
                .arguments
                .iter()
                .enumerate()
                .position(|(i, a)| !used[i] && a.name.is_none());

            if let Some(i) = named.or(positional) {
                used[i] = true;
                let arg = &call.arguments[i];
                let tag = self.determine_element_tag(&arg.value);
                let op = self.generate_expression(&arg.value)?;
                field_ops.push((field.name.clone(), op, tag));
            } else if let Some(default) = &field.default {
                let tag = self.determine_element_tag(default);
                let op = self.generate_expression(default)?;
                field_ops.push((field.name.clone(), op, tag));
            } else {
                // 可选字段缺省 -> null
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::Const {
                        dst: local,
                        value: IrValue::Null,
                    });
                field_ops.push((field.name.clone(), Operand::Local(local), 6));
            }
        }

        let local = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::NewObject {
                dst: local,
                fields: field_ops,
            });
        Ok(Operand::Local(local))
    }
}
