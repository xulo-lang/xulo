use super::*;

impl IrGenerator {
    /// 记录模块级 `const`：标量使用点内联初始化表达式，聚合值走惰性槽
    /// （首次引用时物化一次，之后共享同一份堆值）。
    pub(super) fn collect_const(&mut self, module_id: &str, b: &LetBinding) {
        if !b.is_const {
            return;
        }
        let Some(expr) = &b.value else { return };
        let key = self.resolve_value(&b.name);
        let slot = self.const_slot_count;
        self.const_slot_count += 1;
        self.consts.insert(
            key,
            ConstDef {
                module_id: module_id.to_string(),
                tag: Self::const_static_tag(expr),
                ty: b
                    .type_annotation
                    .as_ref()
                    .map(|ty| self.canonical_type(ty))
                    .or_else(|| Self::const_static_type(expr).map(|ty| self.canonical_type(&ty))),
                expr: expr.clone(),
                slot,
            },
        );
    }

    /// 字面量（含取负）的静态标签
    pub(super) fn const_static_tag(expr: &Expression) -> i64 {
        match expr {
            Expression::Literal { value, .. } => match value {
                Literal::String(_) => 0,
                Literal::Int(n) if *n > i64::MAX as i128 => 10,
                Literal::Int(_) => 1,
                Literal::Float(_) => 2,
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
                Literal::Int(_) | Literal::Float(_) => Type::Number,
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

    /// 常量是否为聚合值（堆指针）。聚合常量物化一次后所有引用共享同一份
    /// 值（与解释器把 `const` 绑定求值一次一致），避免每次读取都重新分配。
    fn const_is_aggregate(&self, def: &ConstDef) -> bool {
        if matches!(def.tag, 3 | 4 | 7) {
            return true;
        }
        if let Some(ty) = &def.ty
            && self.structural_type_tag(ty).is_some()
        {
            return true;
        }
        self.expr_builds_aggregate(&def.expr)
    }

    /// 初始化表达式是否显式构造聚合值（列表/对象/元组/Map/struct/enum）。
    fn expr_builds_aggregate(&self, expr: &Expression) -> bool {
        match expr {
            Expression::Literal {
                value: Literal::List(_) | Literal::Object(_),
                ..
            }
            | Expression::Tuple { .. }
            | Expression::TypedMap(_)
            | Expression::Range(_) => true,
            Expression::Unary(u)
                if matches!(
                    u.operator,
                    UnaryOperator::Neg | UnaryOperator::Move | UnaryOperator::Copy
                ) =>
            {
                self.expr_builds_aggregate(&u.operand)
            }
            Expression::EnumRef(e) => self.enum_variant_index(&e.enum_name, &e.variant).is_some(),
            Expression::Call(call) if call.object.is_none() => {
                if matches!(
                    call.callee.as_str(),
                    "Map::from_entries" | "Set::from_list" | "Array::filled"
                ) {
                    return true;
                }
                if self.structs.contains_key(&self.resolve_type(&call.callee)) {
                    return true;
                }
                call.callee
                    .split_once("::")
                    .is_some_and(|(name, variant)| self.enum_variant_index(name, variant).is_some())
            }
            _ => false,
        }
    }

    /// 入口常量初始化段：按声明序把每个聚合常量物化到自己的槽里。
    /// 段内相互引用走惰性初始化，函数体里的引用只读槽。
    pub(super) fn emit_const_initializers(&mut self) -> Result<(), XuloError> {
        let entry = self.module.entry_point.0;
        self.current_func = Some(entry);
        self.emitting_const_init = true;
        self.locals.clear();
        self.local_types.clear();
        self.source_types.clear();
        self.uint_range_names.clear();
        self.label_map.clear();

        let mut defs: Vec<(String, ConstDef)> = Vec::new();
        for (name, def) in &self.consts {
            let saved = std::mem::replace(&mut self.current_module, def.module_id.clone());
            let aggregate = self.const_is_aggregate(def);
            self.current_module = saved;
            if aggregate {
                defs.push((name.clone(), def.clone()));
            }
        }
        defs.sort_by_key(|(_, def)| def.slot);

        let body_start = self.module.functions[entry].instructions.len();
        for (name, def) in defs {
            let saved = std::mem::replace(&mut self.current_module, def.module_id.clone());
            self.generate_const_lazy_init(&name, &def, def.slot)?;
            self.current_module = saved;
        }
        self.emitting_const_init = false;

        let init = self.module.functions[entry]
            .instructions
            .split_off(body_start);
        let mut prefix = Vec::with_capacity(init.len() + 1);
        prefix.push(Instruction::RuntimeCall {
            dst: None,
            func: RuntimeFn::ConstReset,
            args: vec![Operand::Const(IrValue::I64(self.const_slot_count))],
        });
        prefix.extend(init);
        prefix.append(&mut self.module.functions[entry].instructions);
        self.module.functions[entry].instructions = prefix;
        Ok(())
    }

    /// 读聚合常量槽，返回承载指针的本地槽（入口已按声明序物化）
    fn emit_const_load(&mut self, slot: i64) -> LocalId {
        let func_idx = self.current_func.unwrap();
        let dst = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(dst),
                func: RuntimeFn::ConstGet,
                args: vec![Operand::Const(IrValue::I64(slot))],
            });
        dst
    }

    /// 读聚合常量槽作为操作数
    fn generate_const_load(&mut self, slot: i64) -> Operand {
        Operand::Local(self.emit_const_load(slot))
    }

    /// 取聚合常量的惰性槽：未初始化时求值一次并写入，之后直接读回。
    /// 只在入口的常量初始化段里使用（常量相互引用时可按需先物化）。
    fn generate_const_lazy_init(
        &mut self,
        name: &str,
        def: &ConstDef,
        slot: i64,
    ) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let slot_operand = Operand::Const(IrValue::I64(slot));

        let loaded = self.emit_const_load(slot);

        let is_empty = self.module.functions[func_idx].add_local(IrType::I64);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Eq {
                dst: is_empty,
                left: Operand::Local(loaded),
                right: Operand::Const(IrValue::I64(0)),
            });

        let init_label = self.module.functions[func_idx].add_label();
        let done_label = self.module.functions[func_idx].add_label();
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Branch {
                condition: Operand::Local(is_empty),
                then_label: init_label,
                else_label: done_label,
            });

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(init_label));
        let value = self.inline_const_expr(name, def)?;
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: None,
                func: RuntimeFn::ConstSet,
                args: vec![slot_operand, value],
            });
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Jump(done_label));

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(done_label));
        Ok(self.generate_const_load(slot))
    }

    /// 内联 const 的初始化表达式：切到定义模块的上下文生成（`use` 绑定/
    /// 模块前缀都按定义处解析），并检测自引用。
    fn inline_const_expr(&mut self, name: &str, def: &ConstDef) -> Result<Operand, XuloError> {
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

    /// 生成一次模块级常量引用：聚合值经惰性槽物化，标量保持内联。
    pub(super) fn generate_const_ref(
        &mut self,
        name: &str,
        def: &ConstDef,
    ) -> Result<Operand, XuloError> {
        let saved = std::mem::replace(&mut self.current_module, def.module_id.clone());
        let aggregate = self.const_is_aggregate(def);
        self.current_module = saved;
        if aggregate {
            let slot = def.slot;
            if self.emitting_const_init {
                return self.generate_const_lazy_init(name, def, slot);
            }
            return Ok(self.generate_const_load(slot));
        }
        self.inline_const_expr(name, def)
    }
}
