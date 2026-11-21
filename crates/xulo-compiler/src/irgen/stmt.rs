use super::*;

impl IrGenerator {
    pub(super) fn generate_statement(&mut self, stmt: &Statement) -> Result<(), XuloError> {
        match stmt {
            Statement::Let(binding) => self.generate_let(binding),
            Statement::Return(r) => self.generate_return(r),
            Statement::Expr(e) => {
                if let Expression::If(if_expr) = &e.expr {
                    self.generate_if(if_expr)
                } else {
                    self.generate_expression(&e.expr)?;
                    Ok(())
                }
            }
            Statement::For(for_stmt) => self.generate_for(for_stmt),
            Statement::While(while_stmt) => self.generate_while(while_stmt),
            Statement::Assign(assign) => self.generate_assign(assign),
            Statement::Fn(f) => self.generate_inner_fn(f),
            Statement::Export(_) => Ok(()), // 已处理
            Statement::Break => {
                if let Some(ctx) = self.loop_stack.last() {
                    let break_label = ctx.break_label;
                    let depth = ctx.defer_depth;
                    self.emit_defers_down_to(depth)?;
                    let func_idx = self.current_func.unwrap();
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Jump(break_label));
                }
                Ok(())
            }
            Statement::Continue => {
                if let Some(ctx) = self.loop_stack.last() {
                    let continue_label = ctx.continue_label;
                    let depth = ctx.defer_depth;
                    self.emit_defers_down_to(depth)?;
                    let func_idx = self.current_func.unwrap();
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Jump(continue_label));
                }
                Ok(())
            }
            // `defer` registers its operand now and runs it when the enclosing
            // block is left (`leave_block`, or before `return`/`break`/`continue`).
            // The operand is not evaluated at registration time, matching the
            // interpreter (spec `statements/return-and-block.md`).
            Statement::Defer(d) => {
                if let Some(block) = self.defer_blocks.last_mut() {
                    block.push(d.body.clone());
                }
                Ok(())
            }
            _ => Ok(()), // 其他语句暂时忽略
        }
    }

    pub(super) fn generate_let(&mut self, binding: &LetBinding) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();

        // 在生成表达式之前，先记录源级类型
        if let Some(ann) = &binding.type_annotation {
            self.source_types.insert(binding.name.clone(), ann.clone());
        } else if let Some(ref expr) = binding.value
            && let Some(src_ty) = self.infer_source_type_from_expr(expr)
        {
            self.source_types.insert(binding.name.clone(), src_ty);
        }
        // A binding initialized with an unsigned-only value prints unsigned;
        // shadowing replaces any earlier classification.
        if binding
            .value
            .as_ref()
            .is_some_and(|e| self.expr_has_unsigned_tag(e))
        {
            self.uint_range_names.insert(binding.name.clone());
        } else {
            self.uint_range_names.remove(&binding.name);
        }

        let boxed = self.boxed_names.contains(&binding.name);

        // 自捕获闭包（`let f = fn(){ f() }`）：cell 必须在初始化表达式
        // 求值前就位，内部字面量才能捕获到 cell 指针
        if boxed
            && binding
                .value
                .as_ref()
                .is_some_and(|e| matches!(e, Expression::FnExpr(_) | Expression::Arrow(_)))
        {
            let ty = binding
                .type_annotation
                .as_ref()
                .map(|ann| self.type_to_ir(Some(ann)))
                .unwrap_or(IrType::Pointer);
            let cell = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(cell),
                    func: RuntimeFn::AllocCell,
                    args: vec![Operand::Const(IrValue::Null)],
                });
            self.locals.insert(binding.name.clone(), cell);
            self.local_types.insert(cell, IrType::Pointer);
            self.boxed_logical.insert(binding.name.clone(), ty);

            let init_val = self.generate_expression(binding.value.as_ref().unwrap())?;
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: None,
                    func: RuntimeFn::CellSet,
                    args: vec![Operand::Local(cell), init_val],
                });
            return Ok(());
        }

        // 先生成表达式以获取实际类型
        let init_val = if let Some(ref expr) = binding.value {
            self.generate_expression(expr)?
        } else {
            Operand::Const(IrValue::Null)
        };

        // `Array<T, N>` shares the list representation; an object literal
        // annotated with a struct type is a struct, not a `Map`. Register both
        // the value category and the source type name (spec
        // `memory-and-runtime.md`; dynamic trait dispatch needs the name).
        let init_val = match &binding.type_annotation {
            Some(Type::Named(name, _)) if name == "Array" => {
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: None,
                        func: RuntimeFn::MarkValueArray,
                        args: vec![init_val.clone()],
                    });
                init_val
            }
            Some(Type::Named(name, _)) => {
                let resolved = self.resolve_type(name);
                if self.structs.contains_key(&resolved) {
                    let name_idx = self.module.add_string(resolved);
                    let name_local = self.module.functions[func_idx].add_local(IrType::Pointer);
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Const {
                            dst: name_local,
                            value: IrValue::String(name_idx),
                        });
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::RuntimeCall {
                            dst: None,
                            func: RuntimeFn::MarkStructTyped,
                            args: vec![init_val.clone(), Operand::Local(name_local)],
                        });
                }
                init_val
            }
            _ => init_val,
        };
        // `let x = e`: a value-category aggregate gets its own container;
        // reference-category values keep sharing (R8).
        let init_val = if let Some(expr) = &binding.value {
            let tag = self.tag_operand_for(expr, &init_val);
            self.shallow_bind_copy(init_val, tag)
        } else {
            init_val
        };

        // 元组/对象解构：`let (a, b) = expr` / `let { f, g: h } = expr`
        if binding.tuple_names.is_some() || binding.object_destructuring.is_some() {
            return self.generate_destructured_lets(binding, init_val, func_idx);
        }

        // 推断类型
        let ty = if let Some(ann) = &binding.type_annotation {
            // number 绑定若初始化是 f64（小字面量/浮点运算），槽位用 F64
            // 承载，打印与后续算术都按小数走
            if matches!(ann, Type::Number)
                && binding
                    .value
                    .as_ref()
                    .is_some_and(|e| self.determine_element_tag(e) == 2)
            {
                IrType::F64
            } else {
                self.type_to_ir(Some(ann))
            }
        } else {
            // 根据表达式推断类型
            self.infer_type_from_operand(&init_val, func_idx)
        };

        if boxed {
            // 被嵌套字面量捕获的绑定：本地槽持有 cell 指针，值写入 cell。
            // 逻辑类型保留在 boxed_logical 里供读出/标签使用
            let cell = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(cell),
                    func: RuntimeFn::AllocCell,
                    args: vec![init_val],
                });
            self.locals.insert(binding.name.clone(), cell);
            self.local_types.insert(cell, IrType::Pointer);
            self.boxed_logical.insert(binding.name.clone(), ty);
            return Ok(());
        }

        let local = self.module.functions[func_idx].add_local(ty.clone());
        self.locals.insert(binding.name.clone(), local);
        self.local_types.insert(local, ty.clone());
        if let Operand::Local(source) = &init_val
            && let Some(tag) = self.value_tags.get(source).cloned()
        {
            self.value_tags.insert(local, tag);
        }

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::SetLocal {
                dst: local,
                src: init_val,
            });

        Ok(())
    }

    /// 元组/对象解构绑定：右端求值一次，逐名字 GetIndex / GetField 摘出。
    /// 元素槽用 Pointer 承载，打印端按 tag=-1 运行时检测。
    pub(super) fn generate_destructured_lets(
        &mut self,
        binding: &LetBinding,
        init_val: Operand,
        func_idx: usize,
    ) -> Result<(), XuloError> {
        if let Some(names) = &binding.tuple_names {
            for (i, name) in names.iter().enumerate() {
                let elem = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::GetIndex {
                        dst: elem,
                        array: init_val.clone(),
                        index: Operand::Const(IrValue::I64(i as i64)),
                    });
                let elem =
                    self.shallow_bind_copy(Operand::Local(elem), Operand::Const(IrValue::I64(-1)));
                if let Operand::Local(local) = elem {
                    self.locals.insert(name.clone(), local);
                }
            }
            return Ok(());
        }
        if let Some(fields) = &binding.object_destructuring {
            for (field, alias) in fields {
                let name = alias.clone().unwrap_or_else(|| field.clone());
                let elem = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::GetField {
                        dst: elem,
                        object: init_val.clone(),
                        field: field.clone(),
                    });
                let elem =
                    self.shallow_bind_copy(Operand::Local(elem), Operand::Const(IrValue::I64(-1)));
                if let Operand::Local(local) = elem {
                    self.locals.insert(name, local);
                }
            }
            return Ok(());
        }
        Ok(())
    }

    /// Binding/assignment copy (spec `memory-and-runtime.md`, value semantics):
    /// the runtime duplicates `struct`/`enum`/`Array`/tuple containers and
    /// leaves reference-category values shared. Scalar slots need no copy, so
    /// only pointer-shaped operands pay for the call.
    pub(super) fn shallow_bind_copy(&mut self, value: Operand, tag: Operand) -> Operand {
        let func_idx = self.current_func.unwrap();
        if !matches!(self.operand_slot_type(&value), IrType::Pointer) {
            return value;
        }
        // A dynamic tag attached to the source local (JSON index payloads,
        // match bindings) must follow the copy, or printing the binding falls
        // back to the static slot tag.
        let dynamic_tag = match &value {
            Operand::Local(source) => self.value_tags.get(source).cloned(),
            _ => None,
        };
        let local = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(local),
                func: RuntimeFn::ValueShallowCopy,
                args: vec![value, tag],
            });
        if let Some(dynamic_tag) = dynamic_tag {
            self.value_tags.insert(local, dynamic_tag);
        }
        Operand::Local(local)
    }

    pub(super) fn generate_return(&mut self, r: &ReturnStmt) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();

        if let Some(value) = &r.value {
            let operand = self.generate_expression(value)?;
            if self.determine_element_tag(value) == 2 {
                self.emit_register_float(operand.clone());
            }
            self.emit_all_defers()?;
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Return(Some(operand)));
        } else {
            self.emit_all_defers()?;
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Return(None));
        }

        Ok(())
    }

    pub(super) fn generate_if(&mut self, if_expr: &IfExpr) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();
        let condition = self.generate_expression(&if_expr.condition)?;

        let then_label = self.module.functions[func_idx].add_label();
        let else_label = self.module.functions[func_idx].add_label();
        let end_label = self.module.functions[func_idx].add_label();

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Branch {
                condition,
                then_label,
                else_label,
            });

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(then_label));
        let scope = self.enter_block();
        for stmt in &if_expr.then_branch.statements {
            self.generate_statement(stmt)?;
        }
        self.leave_block(scope)?;
        // 分支可能以 break/continue（Jump）或 return 收尾：已有终结指令就不再
        // 追加 Jump(end)，否则同一块里出现双终结指令（codegen 验证器必错）
        let then_has_terminator = self.module.functions[func_idx]
            .instructions
            .last()
            .is_some_and(|i| {
                matches!(
                    i,
                    Instruction::Return(_) | Instruction::Jump(_) | Instruction::Branch { .. }
                )
            });
        if !then_has_terminator {
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Jump(end_label));
        }

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(else_label));
        if let Some(else_branch) = &if_expr.else_branch {
            let scope = self.enter_block();
            for stmt in &else_branch.statements {
                self.generate_statement(stmt)?;
            }
            self.leave_block(scope)?;
        }
        let else_has_terminator = self.module.functions[func_idx]
            .instructions
            .last()
            .is_some_and(|i| {
                matches!(
                    i,
                    Instruction::Return(_) | Instruction::Jump(_) | Instruction::Branch { .. }
                )
            });
        if !else_has_terminator {
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Jump(end_label));
        }

        // Only emit end_label if at least one branch jumps to it
        if !then_has_terminator || !else_has_terminator {
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Label(end_label));
        } else {
            // Both branches returned - emit end_label with a dead return so it's not empty
            let is_void = self.module.functions[func_idx].return_type.is_void();
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Label(end_label));
            if !is_void {
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::Return(Some(Operand::Const(IrValue::I64(0)))));
            } else {
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::Return(None));
            }
        }

        Ok(())
    }

    pub(super) fn generate_for(&mut self, for_stmt: &ForStmt) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();
        let iterable_type = self.infer_source_type_from_expr(&for_stmt.iterable);
        let map_keys = matches!(
            &iterable_type,
            Some(Type::Named(name, _)) if name == "Map"
        ) || matches!(&iterable_type, Some(Type::Object | Type::ObjectType(_)));
        let iterated_type = match &iterable_type {
            Some(Type::List(inner)) => Some((**inner).clone()),
            Some(Type::Named(name, args)) if name == "Map" => {
                Some(args.first().cloned().unwrap_or(Type::String))
            }
            Some(Type::Named(name, args)) if name == "Set" => {
                Some(args.first().cloned().unwrap_or(Type::Any))
            }
            Some(Type::Object | Type::ObjectType(_)) => Some(Type::String),
            _ => None,
        };

        // 创建循环变量（被捕获时本地槽持有 cell；每次迭代换新 cell，
        // 同一轮迭代里创建的闭包共享该轮的值）
        let boxed = self.boxed_names.contains(&for_stmt.iter_var);
        let iter_local = self.module.functions[func_idx].add_local(if boxed {
            IrType::Pointer
        } else {
            IrType::I64
        });
        self.locals.insert(for_stmt.iter_var.clone(), iter_local);
        if boxed {
            let logical = if matches!(for_stmt.iterable, Expression::Range(_)) {
                IrType::I64
            } else if let Some(inner) = &iterated_type {
                self.type_to_ir(Some(inner))
            } else {
                IrType::Pointer
            };
            self.boxed_logical
                .insert(for_stmt.iter_var.clone(), logical);
        }

        // 推断迭代变量的源级类型
        if let Some(inner) = iterated_type {
            self.source_types.insert(for_stmt.iter_var.clone(), inner);
        }

        let start_label = self.module.functions[func_idx].add_label();
        let body_label = self.module.functions[func_idx].add_label();
        let end_label = self.module.functions[func_idx].add_label();
        let continue_label = self.module.functions[func_idx].add_label();

        self.loop_stack.push(LoopContext {
            break_label: end_label,
            continue_label,
            defer_depth: self.defer_blocks.len(),
        });

        // 生成范围循环的 IR
        if let Expression::Range(range) = &for_stmt.iterable {
            let start = self.generate_expression(&range.start)?;
            let end = self.generate_expression(&range.end)?;

            // 初始化循环变量
            if boxed {
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(iter_local),
                        func: RuntimeFn::AllocCell,
                        args: vec![start],
                    });
            } else {
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::SetLocal {
                        dst: iter_local,
                        src: start,
                    });
            }

            // 跳转到循环开始
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Jump(start_label));

            // 循环开始
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Label(start_label));

            // 检查条件（装箱时从 cell 读出当前值）
            let cond_local = self.module.functions[func_idx].add_local(IrType::Bool);
            let cur = if boxed {
                let tmp = self.module.functions[func_idx].add_local(IrType::I64);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(tmp),
                        func: RuntimeFn::CellGet,
                        args: vec![Operand::Local(iter_local)],
                    });
                Operand::Local(tmp)
            } else {
                Operand::Local(iter_local)
            };
            if range.end_inclusive {
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::Lte {
                        dst: cond_local,
                        left: cur.clone(),
                        right: end,
                    });
            } else {
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::Lt {
                        dst: cond_local,
                        left: cur.clone(),
                        right: end,
                    });
            }

            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Branch {
                    condition: Operand::Local(cond_local),
                    then_label: body_label,
                    else_label: end_label,
                });

            // 循环体
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Label(body_label));
            let scope = self.enter_block();
            for stmt in &for_stmt.body.statements {
                self.generate_statement(stmt)?;
            }
            self.leave_block(scope)?;

            // 跳转到 continue 标签
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Jump(continue_label));

            // continue 标签
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Label(continue_label));

            // 递增
            let one = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Const {
                    dst: one,
                    value: IrValue::I64(1),
                });
            let new_val = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Add {
                    dst: new_val,
                    left: cur,
                    right: Operand::Local(one),
                });
            if boxed {
                // 换新 cell：本轮迭代内创建的闭包捕获本轮的值
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(iter_local),
                        func: RuntimeFn::AllocCell,
                        args: vec![Operand::Local(new_val)],
                    });
            } else {
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::SetLocal {
                        dst: iter_local,
                        src: Operand::Local(new_val),
                    });
            }

            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Jump(start_label));
        } else {
            // Generic iteration over any expression (array or variable)
            let array_operand = if map_keys {
                let keys = self.module.functions[func_idx].add_local(IrType::Pointer);
                let map = self.generate_expression(&for_stmt.iterable)?;
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(keys),
                        func: RuntimeFn::ObjectKeys,
                        args: vec![map],
                    });
                Operand::Local(keys)
            } else {
                self.generate_expression(&for_stmt.iterable)?
            };
            let array_local = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::SetLocal {
                    dst: array_local,
                    src: array_operand,
                });

            // Get array length
            let len_local = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(len_local),
                    func: RuntimeFn::ArrayLen,
                    args: vec![Operand::Local(array_local)],
                });

            // index = 0
            let index_local = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Const {
                    dst: index_local,
                    value: IrValue::I64(0),
                });

            // Jump to start
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Jump(start_label));

            // Loop start: check index < len
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Label(start_label));
            let cond_local = self.module.functions[func_idx].add_local(IrType::Bool);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Lt {
                    dst: cond_local,
                    left: Operand::Local(index_local),
                    right: Operand::Local(len_local),
                });
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Branch {
                    condition: Operand::Local(cond_local),
                    then_label: body_label,
                    else_label: end_label,
                });

            // Body
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Label(body_label));
            // iter_var = array[index]（装箱时每元素换新 cell）
            if boxed {
                let raw = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::GetIndex {
                        dst: raw,
                        array: Operand::Local(array_local),
                        index: Operand::Local(index_local),
                    });
                let copied =
                    self.shallow_bind_copy(Operand::Local(raw), Operand::Const(IrValue::I64(-1)));
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(iter_local),
                        func: RuntimeFn::AllocCell,
                        args: vec![copied],
                    });
            } else {
                let raw = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::GetIndex {
                        dst: raw,
                        array: Operand::Local(array_local),
                        index: Operand::Local(index_local),
                    });
                let copied =
                    self.shallow_bind_copy(Operand::Local(raw), Operand::Const(IrValue::I64(-1)));
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::SetLocal {
                        dst: iter_local,
                        src: copied,
                    });
            }
            let scope = self.enter_block();
            for stmt in &for_stmt.body.statements {
                self.generate_statement(stmt)?;
            }
            self.leave_block(scope)?;

            // Jump to continue
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Jump(continue_label));

            // Continue: index++
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Label(continue_label));
            let one = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Const {
                    dst: one,
                    value: IrValue::I64(1),
                });
            let new_index = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Add {
                    dst: new_index,
                    left: Operand::Local(index_local),
                    right: Operand::Local(one),
                });
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::SetLocal {
                    dst: index_local,
                    src: Operand::Local(new_index),
                });
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Jump(start_label));
        }

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(end_label));
        self.loop_stack.pop();

        Ok(())
    }

    pub(super) fn generate_while(&mut self, while_stmt: &WhileStmt) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();

        let start_label = self.module.functions[func_idx].add_label();
        let body_label = self.module.functions[func_idx].add_label();
        let end_label = self.module.functions[func_idx].add_label();
        let continue_label = self.module.functions[func_idx].add_label();

        self.loop_stack.push(LoopContext {
            break_label: end_label,
            continue_label,
            defer_depth: self.defer_blocks.len(),
        });

        // 循环开始：先补一条跳转终结前一块（codegen 的 Label 会开新块）
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Jump(start_label));
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(start_label));

        // 检查条件
        let condition = self.generate_expression(&while_stmt.condition)?;
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Branch {
                condition,
                then_label: body_label,
                else_label: end_label,
            });

        // 循环体
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(body_label));
        let scope = self.enter_block();
        for stmt in &while_stmt.body.statements {
            self.generate_statement(stmt)?;
        }
        self.leave_block(scope)?;
        // 语句块可能以非终结指令收尾（如末尾是赋值），补跳转接住 fall-through；
        // 已以 break/return 等终结则不再追加，避免双终结指令
        let last_is_terminator = self.module.functions[func_idx]
            .instructions
            .last()
            .is_some_and(|i| {
                matches!(
                    i,
                    Instruction::Jump(_) | Instruction::Branch { .. } | Instruction::Return(_)
                )
            });
        if !last_is_terminator {
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Jump(continue_label));
        }

        // continue 标签
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(continue_label));
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Jump(start_label));

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(end_label));
        self.loop_stack.pop();

        Ok(())
    }

    pub(super) fn generate_assign(&mut self, assign: &AssignStmt) -> Result<(), XuloError> {
        self.generate_assign_value(assign)?;
        Ok(())
    }

    /// Lower an assignment and return the value it assigns. A chained
    /// right-hand side (`a = b = c`) lowers the inner assignment first and
    /// feeds its value to the outer target.
    pub(super) fn generate_assign_value(
        &mut self,
        assign: &AssignStmt,
    ) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let value = match &assign.value {
            AssignValue::Expr(expr) => self.generate_expression(expr)?,
            AssignValue::Assign(inner) => self.generate_assign_value(inner)?,
        };
        let tag = Self::assign_value_tag(self, &assign.value);
        // Assignment binds by the same value/reference rule as `let` (R8).
        let value = self.shallow_bind_copy(value, Operand::Const(IrValue::I64(tag)));
        let result = value.clone();

        match &assign.target {
            AssignTarget::Name(name) => {
                // Keep the unsigned-print classification in step with the
                // assigned value (`u64` and `int` share the slot).
                let assigned_unsigned = matches!(
                    &assign.value,
                    AssignValue::Expr(expr) if self.expr_has_unsigned_tag(expr)
                );
                if assigned_unsigned {
                    self.uint_range_names.insert(name.clone());
                } else {
                    self.uint_range_names.remove(name);
                }
                if let Some(local) = self.locals.get(name).copied() {
                    if self.boxed_names.contains(name) {
                        // 装箱变量：写入既有 cell（别名的闭包共享同一 cell）
                        self.module.functions[func_idx].instructions.push(
                            Instruction::RuntimeCall {
                                dst: None,
                                func: RuntimeFn::CellSet,
                                args: vec![Operand::Local(local), value],
                            },
                        );
                    } else {
                        self.module.functions[func_idx]
                            .instructions
                            .push(Instruction::SetLocal {
                                dst: local,
                                src: value,
                            });
                    }
                }
            }
            AssignTarget::Member(obj, field) => {
                let obj_operand = self.generate_expression(obj)?;
                // `p.0 = v` writes a tuple element by position; a tuple is a
                // list at run time.
                if matches!(self.infer_source_type_from_expr(obj), Some(Type::Tuple(_)))
                    && let Ok(index) = field.parse::<usize>()
                {
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::SetIndex {
                            array: obj_operand,
                            index: Operand::Const(IrValue::I64(index as i64)),
                            value,
                            tag,
                        });
                } else {
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::SetField {
                            object: obj_operand,
                            field: field.clone(),
                            value,
                            tag,
                        });
                }
            }
            AssignTarget::Index(obj, idx) => {
                let obj_operand = self.generate_expression(obj)?;
                let idx_operand = self.generate_expression(idx)?;
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::SetIndex {
                        array: obj_operand,
                        index: idx_operand,
                        value,
                        tag,
                    });
            }
        }

        Ok(result)
    }

    fn assign_value_tag(&self, value: &AssignValue) -> i64 {
        match value {
            AssignValue::Expr(expr) => self.determine_element_tag(expr),
            AssignValue::Assign(inner) => self.assign_value_tag(&inner.value),
        }
    }
}
