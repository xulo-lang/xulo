use super::*;

impl IrGenerator {
    /// `expr?`：expr 求值为 Result；Err 时提前 `Return` 该 Err 枚举值
    /// （调用方函数返回类型同为 Result），Ok 时取出 payload 继续
    pub(super) fn generate_propagate(&mut self, expr: &Expression) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let scrut = self.generate_expression(expr)?;
        let scrut_local = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::SetLocal {
                dst: scrut_local,
                src: scrut,
            });
        let scrut_op = Operand::Local(scrut_local);

        let err_idx = self.enum_variant_index("Result", "Err").unwrap_or(1) as i64;
        let vt = self.module.functions[func_idx].add_local(IrType::I64);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(vt),
                func: RuntimeFn::EnumVariant,
                args: vec![scrut_op.clone()],
            });
        let is_err = self.module.functions[func_idx].add_local(IrType::Bool);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Eq {
                dst: is_err,
                left: Operand::Local(vt),
                right: Operand::Const(IrValue::I64(err_idx)),
            });
        let err_label = self.module.functions[func_idx].add_label();
        let ok_label = self.module.functions[func_idx].add_label();
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Branch {
                condition: Operand::Local(is_err),
                then_label: err_label,
                else_label: ok_label,
            });

        // Err：整个枚举值即调用方要传播的结果
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(err_label));
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Return(Some(scrut_op.clone())));

        // Ok：摘 payload 继续求值
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(ok_label));
        let payload = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(payload),
                func: RuntimeFn::EnumGet,
                args: vec![scrut_op, Operand::Const(IrValue::I64(0))],
            });
        Ok(Operand::Local(payload))
    }

    pub(super) fn generate_expression(&mut self, expr: &Expression) -> Result<Operand, XuloError> {
        match expr {
            Expression::Literal { value, .. } => self.generate_literal(value),
            Expression::Identifier { name, .. } => {
                if let Some(local) = self.locals.get(name).copied() {
                    if self.boxed_names.contains(name) {
                        // 装箱变量：从 cell 读出（f64 位模式走专用读回）
                        let logical = self.boxed_logical.get(name).cloned().unwrap_or(IrType::I64);
                        let dst = self.module.functions[self.current_func.unwrap()]
                            .add_local(logical.clone());
                        let func = if matches!(logical, IrType::F64) {
                            RuntimeFn::CellGetF64
                        } else {
                            RuntimeFn::CellGet
                        };
                        self.module.functions[self.current_func.unwrap()]
                            .instructions
                            .push(Instruction::RuntimeCall {
                                dst: Some(dst),
                                func,
                                args: vec![Operand::Local(local)],
                            });
                        return Ok(Operand::Local(dst));
                    }
                    Ok(Operand::Local(local))
                } else if let Some(def) = self.const_def(name).cloned() {
                    // 模块级 const：在定义模块的上下文里内联初始化表达式
                    self.generate_const_ref(name, &def)
                } else if let Some(value) = Self::bare_math_const(name) {
                    // `use std::math::*` 导入的裸名常量（locals 优先，遮蔽已被
                    // 语义层的重名检查排除）
                    let func_idx = self.current_func.unwrap();
                    let local = self.module.functions[func_idx].add_local(IrType::F64);
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Const {
                            dst: local,
                            value: IrValue::F64(value),
                        });
                    Ok(Operand::Local(local))
                } else if self
                    .module
                    .functions
                    .iter()
                    .any(|f| f.name == self.resolve_value(name))
                {
                    // 顶层函数引用作函数值：经 thunk 适配成闭包（名字按模块解析）
                    let resolved = self.resolve_value(name);
                    self.generate_func_value(&resolved)
                } else {
                    Err(XuloError::new(
                        ErrorKind::Semantic,
                        format!("undefined variable: {}", name),
                    ))
                }
            }
            Expression::FnExpr(f) => {
                self.generate_closure_value(&f.params, f.return_type.as_ref(), &f.body, None)
            }
            Expression::Arrow(a) => {
                let (body, expr) = match &a.body {
                    ArrowBody::Expr(e) => (
                        Block {
                            statements: Vec::new(),
                        },
                        Some(e),
                    ),
                    ArrowBody::Block(b) => (b.clone(), None),
                };
                self.generate_closure_value(&a.params, a.return_type.as_ref(), &body, expr)
            }
            // `await e`：编译路径的 async 调用本身同步完成，等待就是取
            // 已经算好的值（`std::time::sleep` 在运行时真实睡眠）。
            Expression::Await { expr, .. } => self.generate_expression(expr),
            Expression::CallValue(cv) => {
                let callee = self.generate_expression(&cv.callee)?;
                let mut args = Vec::new();
                for arg in &cv.arguments {
                    args.push(self.generate_expression(&arg.value)?);
                }
                self.generate_call_indirect(callee, args)
            }
            Expression::BinaryOp(binop) => self.generate_binary_op(binop),
            Expression::Unary(unary) => self.generate_unary(unary),
            Expression::Call(call) => self.generate_call(call),
            Expression::If(if_expr) => self.generate_if_expr(if_expr),
            Expression::Ternary(ternary) => self.generate_ternary(ternary),
            Expression::Nullish(nullish) => self.generate_nullish(nullish),
            Expression::Member(member) => self.generate_member_access(member),
            Expression::Index(index) => self.generate_index(index),
            // `std::math` 常量：F64 槽常量（打印端按 tag=2 还原）
            Expression::EnumRef(e)
                if e.enum_name == "std::math"
                    && matches!(e.variant.as_str(), "PI" | "E" | "TAU" | "INFINITY" | "NAN") =>
            {
                let value = match e.variant.as_str() {
                    "PI" => std::f64::consts::PI,
                    "E" => std::f64::consts::E,
                    "TAU" => std::f64::consts::TAU,
                    "INFINITY" => f64::INFINITY,
                    "NAN" => f64::NAN,
                    _ => unreachable!(),
                };
                let func_idx = self.current_func.unwrap();
                let local = self.module.functions[func_idx].add_local(IrType::F64);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::Const {
                        dst: local,
                        value: IrValue::F64(value),
                    });
                Ok(Operand::Local(local))
            }
            // 无 payload 的枚举变体引用：`Direction::East`；`m::PI` 这类
            // 命名空间成员引用按模块解析成常量/函数值
            Expression::EnumRef(e) => {
                let namespace_target = self
                    .ctx()
                    .and_then(|ctx| ctx.namespaces.get(&e.enum_name).cloned());
                if let Some(target) = namespace_target {
                    let canonical = Self::join_item(&target, &e.variant);
                    if let Some(def) = self.consts.get(&canonical).cloned() {
                        return self.generate_const_ref(&canonical, &def);
                    }
                    if self.module.functions.iter().any(|f| f.name == canonical) {
                        return self.generate_func_value(&canonical);
                    }
                    return Err(XuloError::new(
                        ErrorKind::Semantic,
                        format!("undefined variable: {}::{}", e.enum_name, e.variant),
                    ));
                }
                self.generate_enum_construct(&e.enum_name, &e.variant, &[])
            }
            Expression::Match(m) => self.generate_match(m),
            Expression::Is(test) => self.generate_is(test),
            Expression::Propagate { expr, .. } => self.generate_propagate(expr),
            Expression::Panic { expr, .. } => {
                let value = self.generate_expression(expr)?;
                let tag = self.determine_element_tag(expr);
                let func_idx = self.current_func.unwrap();
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: None,
                        func: RuntimeFn::Panic,
                        args: vec![value, Operand::Const(IrValue::I64(tag))],
                    });
                // panic 不返回值；同 generate_call 的无返回调用，返回一个不会被使用的虚拟值
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                Ok(Operand::Local(local))
            }
            // A tuple literal is a fixed-arity list at run time; positional
            // access and destructuring both read it as one (spec
            // `types/composite-types.md`, "Tuples").
            Expression::Tuple { items, .. } => {
                let func_idx = self.current_func.unwrap();
                let mut elements = Vec::new();
                for item in items {
                    let tag = self.determine_element_tag(item);
                    elements.push((self.generate_expression(item)?, tag));
                }
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::NewArray {
                        dst: local,
                        elements,
                    });
                // Tuples are value category; register the array so a later
                // binding copy detaches it from `List` reference semantics.
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: None,
                        func: RuntimeFn::MarkValueArray,
                        args: vec![Operand::Local(local)],
                    });
                Ok(Operand::Local(local))
            }
            Expression::TypedMap(map) => {
                let func_idx = self.current_func.unwrap();
                let object = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(object),
                        func: RuntimeFn::AllocObject,
                        args: vec![Operand::Const(IrValue::I64(map.entries.len() as i64))],
                    });
                for entry in &map.entries {
                    let key = self.generate_map_key(&entry.key)?;
                    let value = self.generate_expression(&entry.value)?;
                    let value_tag = self.tag_operand_for(&entry.value, &value);
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::RuntimeCall {
                            dst: None,
                            func: RuntimeFn::ObjectSet,
                            args: vec![Operand::Local(object), key, value, value_tag],
                        });
                }
                Ok(Operand::Local(object))
            }
            // 区间表达式 `0..<3` / `1..=4` 求值为列表（for 循环已在
            // generate_for 单独降级，不会走到这里）
            Expression::Range(r) => {
                let start = self.generate_expression(&r.start)?;
                let end = self.generate_expression(&r.end)?;
                let func_idx = self.current_func.unwrap();
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(local),
                        func: RuntimeFn::RangeList,
                        args: vec![
                            start,
                            end,
                            Operand::Const(IrValue::I64(if r.end_inclusive { 1 } else { 0 })),
                        ],
                    });
                Ok(Operand::Local(local))
            }
            _ => {
                // 其他表达式返回 null
                let func_idx = self.current_func.unwrap();
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::Const {
                        dst: local,
                        value: IrValue::Null,
                    });
                Ok(Operand::Local(local))
            }
        }
    }

    /// Lower one written const argument to an `i64` operand: an integer
    /// literal, a const parameter (a local), or a module-level `const` whose
    /// initializer is inlined (spec `types/generics.md`).
    fn generate_const_argument(&mut self, arg: &Type) -> Result<Operand, XuloError> {
        match arg {
            Type::Literal(text) => {
                let n: i64 = text.parse().map_err(|_| {
                    XuloError::new(
                        ErrorKind::Semantic,
                        format!("`{text}` is not a const argument"),
                    )
                })?;
                let func_idx = self.current_func.unwrap();
                let local = self.module.functions[func_idx].add_local(IrType::I64);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::Const {
                        dst: local,
                        value: IrValue::I64(n),
                    });
                Ok(Operand::Local(local))
            }
            Type::Named(name, args) if args.is_empty() => {
                if let Some(&local) = self.locals.get(name) {
                    return Ok(Operand::Local(local));
                }
                if self.const_def(name).is_some() {
                    let expr = Expression::Identifier {
                        name: name.clone(),
                        span: 0..0,
                    };
                    return self.generate_expression(&expr);
                }
                Err(XuloError::new(
                    ErrorKind::Semantic,
                    format!("`{name}` is not a const value"),
                ))
            }
            other => Err(XuloError::new(
                ErrorKind::Semantic,
                format!("{} is not a const argument", other.display()),
            )),
        }
    }

    pub(super) fn generate_literal(&mut self, lit: &Literal) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let (value, ty) = match lit {
            // Integer spelling (`42`, `0xff`) stays in an I64 slot; any literal
            // written with a decimal point (`3.0`) is a float and must use an
            // F64 slot even when integral, so `a / 1.0` is float division.
            Literal::Int(n) => (IrValue::I64(*n as i64), IrType::I64),
            Literal::Float(n) => (IrValue::F64(*n), IrType::F64),
            Literal::Boolean(b) => (IrValue::Bool(*b), IrType::Bool),
            Literal::String(s) => {
                let idx = self.module.add_string(s.clone());
                (IrValue::String(idx), IrType::Pointer)
            }
            Literal::Null => (IrValue::Null, IrType::Pointer),
            Literal::List(elements) => {
                if !elements
                    .iter()
                    .any(|elem| matches!(elem, Expression::Spread { .. }))
                {
                    let mut element_ops = Vec::new();
                    for elem in elements {
                        let tag = self.determine_element_tag(elem);
                        element_ops.push((self.generate_expression(elem)?, tag));
                    }
                    return Ok(self.emit_new_array(element_ops));
                }
                // Spread 需要按出现顺序拼接：静态元素先聚成一段数组，再与
                // 展开操作数依次做运行时连接。起始空数组保证结果不与任何
                // 操作数共享。
                let mut accumulator = self.emit_new_array(Vec::new());
                let mut run = Vec::new();
                for elem in elements {
                    if let Expression::Spread { expr, .. } = elem {
                        if !run.is_empty() {
                            let segment = self.emit_new_array(std::mem::take(&mut run));
                            accumulator = self.emit_list_concat(accumulator, segment);
                        }
                        let spread = self.generate_expression(expr)?;
                        accumulator = self.emit_list_concat(accumulator, spread);
                    } else {
                        let tag = self.determine_element_tag(elem);
                        run.push((self.generate_expression(elem)?, tag));
                    }
                }
                if !run.is_empty() {
                    let segment = self.emit_new_array(run);
                    accumulator = self.emit_list_concat(accumulator, segment);
                }
                return Ok(accumulator);
            }
            Literal::Object(fields) => {
                if !fields
                    .iter()
                    .any(|field| matches!(field, ObjectField::Spread { .. }))
                {
                    let mut field_ops = Vec::new();
                    for field in fields {
                        if let ObjectField::Field { name, value } = field {
                            let tag = self.determine_element_tag(value);
                            field_ops.push((name.clone(), self.generate_expression(value)?, tag));
                        }
                    }
                    return Ok(self.emit_new_object(field_ops));
                }
                let accumulator = self.emit_new_object(Vec::new());
                let mut run = Vec::new();
                for field in fields {
                    match field {
                        ObjectField::Field { name, value } => {
                            let tag = self.determine_element_tag(value);
                            run.push((name.clone(), self.generate_expression(value)?, tag));
                        }
                        ObjectField::Spread { value } => {
                            if !run.is_empty() {
                                let segment = self.emit_new_object(std::mem::take(&mut run));
                                self.emit_object_extend(accumulator.clone(), segment);
                            }
                            let spread = self.generate_expression(value)?;
                            self.emit_object_extend(accumulator.clone(), spread);
                        }
                    }
                }
                if !run.is_empty() {
                    let segment = self.emit_new_object(run);
                    self.emit_object_extend(accumulator.clone(), segment);
                }
                return Ok(accumulator);
            }
        };

        let local = self.module.functions[func_idx].add_local(ty);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Const { dst: local, value });
        Ok(Operand::Local(local))
    }

    fn emit_new_array(&mut self, elements: Vec<(Operand, i64)>) -> Operand {
        let func_idx = self.current_func.unwrap();
        let local = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::NewArray {
                dst: local,
                elements,
            });
        Operand::Local(local)
    }

    fn emit_list_concat(&mut self, left: Operand, right: Operand) -> Operand {
        let func_idx = self.current_func.unwrap();
        let local = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(local),
                func: RuntimeFn::ArrayConcat,
                args: vec![left, right],
            });
        Operand::Local(local)
    }

    fn emit_new_object(&mut self, fields: Vec<(String, Operand, i64)>) -> Operand {
        let func_idx = self.current_func.unwrap();
        let local = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::NewObject { dst: local, fields });
        Operand::Local(local)
    }

    fn emit_object_extend(&mut self, target: Operand, source: Operand) {
        let func_idx = self.current_func.unwrap();
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: None,
                func: RuntimeFn::ObjectExtend,
                args: vec![target, source],
            });
    }

    pub(super) fn generate_binary_op(&mut self, binop: &BinaryOp) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let left = self.generate_expression(&binop.left)?;
        let right = self.generate_expression(&binop.right)?;

        // 字符串/列表拼接要在 dyn 数值归一之前定型：match 绑定的 payload
        // 名字在 `dyn_tag_of` 里，若先经 `xulo_as_f64` 变成 f64，拼接分支
        // 再把它当指针解引用就会崩（或把两个指针相加后当字符串）。
        let string_add = matches!(binop.operator, BinaryOperator::Add)
            && (self.is_string_like(&binop.left) || self.is_string_like(&binop.right));
        let array_add = matches!(binop.operator, BinaryOperator::Add)
            && (self.is_array_like_expr(&binop.left) || self.is_array_like_expr(&binop.right));

        // match 绑定参与的数值运算：payload 的 int/f64 标签运行时才知道，
        // 两侧先经 xulo_as_f64 归一到 f64
        let dyn_math = !(string_add || array_add)
            && matches!(
                binop.operator,
                BinaryOperator::Add
                    | BinaryOperator::Sub
                    | BinaryOperator::Mul
                    | BinaryOperator::Div
                    | BinaryOperator::Lt
                    | BinaryOperator::Gt
                    | BinaryOperator::Lte
                    | BinaryOperator::Gte
            )
            && (self.expr_is_dyn(&binop.left) || self.expr_is_dyn(&binop.right));
        let (left, right) = if dyn_math {
            let left = match &binop.left {
                Expression::Identifier { name, .. } if self.is_dyn_name(name) => {
                    self.as_f64_operand(left, name)?
                }
                _ => left,
            };
            let right = match &binop.right {
                Expression::Identifier { name, .. } if self.is_dyn_name(name) => {
                    self.as_f64_operand(right, name)?
                }
                _ => right,
            };
            (left, right)
        } else {
            (left, right)
        };

        // String/array concatenation is decided by source-level types, not the
        // IR slot type: struct fields are Pointer slots that hold numbers, so
        // `self.x + dx` must lower to integer Add, not StringConcat.
        if string_add {
            let local = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func: RuntimeFn::StringConcat,
                    args: vec![left, right],
                });
            return Ok(Operand::Local(local));
        }

        if array_add {
            let local = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func: RuntimeFn::ArrayConcat,
                    args: vec![left, right],
                });
            return Ok(Operand::Local(local));
        }

        // `string == string` compares contents. A raw `icmp` on the two
        // pointers only works for interned literals; runtime-produced strings
        // (argv, file contents, slices) need the content comparison.
        if matches!(binop.operator, BinaryOperator::Eq | BinaryOperator::Neq)
            && (self.is_string_like(&binop.left) || self.is_string_like(&binop.right))
        {
            let eq = self.module.functions[func_idx].add_local(IrType::Bool);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(eq),
                    func: RuntimeFn::StringEqual,
                    args: vec![left, right],
                });
            if matches!(binop.operator, BinaryOperator::Neq) {
                let negated = self.module.functions[func_idx].add_local(IrType::Bool);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::Not {
                        dst: negated,
                        operand: Operand::Local(eq),
                    });
                return Ok(Operand::Local(negated));
            }
            return Ok(Operand::Local(eq));
        }

        // `StringBuilder == StringBuilder` compares the current text, not the
        // handle identity (spec `types/composite-types.md`).
        if matches!(binop.operator, BinaryOperator::Eq | BinaryOperator::Neq)
            && (self.expr_is_string_builder(&binop.left)
                || self.expr_is_string_builder(&binop.right))
        {
            let eq = self.module.functions[func_idx].add_local(IrType::Bool);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(eq),
                    func: RuntimeFn::StringBuilderEq,
                    args: vec![left, right],
                });
            if matches!(binop.operator, BinaryOperator::Neq) {
                let negated = self.module.functions[func_idx].add_local(IrType::Bool);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::Not {
                        dst: negated,
                        operand: Operand::Local(eq),
                    });
                return Ok(Operand::Local(negated));
            }
            return Ok(Operand::Local(eq));
        }

        // Structural equality for value-carrying aggregates (spec
        // `expressions/operators.md`, "Equality"). A raw `icmp` compares the
        // handle, which is wrong for two independently built equal lists,
        // tuples, structs, maps, sets, and enums; route them through the
        // runtime's recursive comparison instead.
        if matches!(binop.operator, BinaryOperator::Eq | BinaryOperator::Neq)
            && !self.has_float_operand(&binop.left)
            && !self.has_float_operand(&binop.right)
            && (self.expr_is_structural_value(&binop.left)
                || self.expr_is_structural_value(&binop.right))
        {
            let left_tag = self.structural_tag_operand(&binop.left, &left);
            let right_tag = self.structural_tag_operand(&binop.right, &right);
            let eq = self.module.functions[func_idx].add_local(IrType::Bool);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(eq),
                    func: RuntimeFn::ValueEqual,
                    args: vec![left, left_tag, right, right_tag],
                });
            if matches!(binop.operator, BinaryOperator::Neq) {
                let negated = self.module.functions[func_idx].add_local(IrType::Bool);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::Not {
                        dst: negated,
                        operand: Operand::Local(eq),
                    });
                return Ok(Operand::Local(negated));
            }
            return Ok(Operand::Local(eq));
        }

        // Integral arithmetic is checked at runtime: overflow, division by
        // zero, and an out-of-range shift abort instead of wrapping, matching
        // the interpreter (spec `types/primitive-types.md`, "Overflow").
        // `dyn_math` bindings were promoted to `f64` above, and either float
        // operand keeps the IEEE path.
        if !dyn_math
            && !self.has_float_operand(&binop.left)
            && !self.has_float_operand(&binop.right)
            && let Some(func) = match binop.operator {
                BinaryOperator::Add => Some(RuntimeFn::IntAdd),
                BinaryOperator::Sub => Some(RuntimeFn::IntSub),
                BinaryOperator::Mul => Some(RuntimeFn::IntMul),
                BinaryOperator::Div => Some(RuntimeFn::IntDiv),
                BinaryOperator::Mod => Some(RuntimeFn::IntRem),
                BinaryOperator::Pow => Some(RuntimeFn::IntPow),
                _ => None,
            }
        {
            let unsigned =
                self.expr_is_unsigned_int(&binop.left) || self.expr_is_unsigned_int(&binop.right);
            let local = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func,
                    args: vec![
                        left,
                        right,
                        Operand::Const(IrValue::I64(i64::from(unsigned))),
                    ],
                });
            return Ok(Operand::Local(local));
        }

        // Integral comparisons are exact across signedness: `int` and `u64`
        // are compared by mathematical value (`0 - 1 == u64::MAX` is false),
        // which a raw signed `icmp` cannot do.
        if !dyn_math
            && !self.has_float_operand(&binop.left)
            && !self.has_float_operand(&binop.right)
            && matches!(
                binop.operator,
                BinaryOperator::Eq
                    | BinaryOperator::Neq
                    | BinaryOperator::Lt
                    | BinaryOperator::Gt
                    | BinaryOperator::Lte
                    | BinaryOperator::Gte
            )
            && (self.expr_is_unsigned_int(&binop.left) || self.expr_is_unsigned_int(&binop.right))
        {
            let left_unsigned = self.expr_is_unsigned_int(&binop.left);
            let right_unsigned = self.expr_is_unsigned_int(&binop.right);
            let ordering = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(ordering),
                    func: RuntimeFn::NumCompare,
                    args: vec![
                        left,
                        right,
                        Operand::Const(IrValue::I64(i64::from(left_unsigned))),
                        Operand::Const(IrValue::I64(i64::from(right_unsigned))),
                    ],
                });
            let local = self.module.functions[func_idx].add_local(IrType::Bool);
            let zero = Operand::Const(IrValue::I64(0));
            let instruction = match binop.operator {
                BinaryOperator::Eq => Instruction::Eq {
                    dst: local,
                    left: Operand::Local(ordering),
                    right: zero,
                },
                BinaryOperator::Neq => Instruction::Neq {
                    dst: local,
                    left: Operand::Local(ordering),
                    right: zero,
                },
                BinaryOperator::Lt => Instruction::Lt {
                    dst: local,
                    left: Operand::Local(ordering),
                    right: zero,
                },
                BinaryOperator::Gt => Instruction::Gt {
                    dst: local,
                    left: Operand::Local(ordering),
                    right: zero,
                },
                BinaryOperator::Lte => Instruction::Lte {
                    dst: local,
                    left: Operand::Local(ordering),
                    right: zero,
                },
                BinaryOperator::Gte => Instruction::Gte {
                    dst: local,
                    left: Operand::Local(ordering),
                    right: zero,
                },
                _ => unreachable!("comparison operator matched above"),
            };
            self.module.functions[func_idx]
                .instructions
                .push(instruction);
            return Ok(Operand::Local(local));
        }

        // `u64 >> n` fills with zero, `int >> n` with the sign bit (spec
        // `expressions/operators.md`); `<<` is the same wrapping operation in
        // both domains.
        if matches!(binop.operator, BinaryOperator::Shr)
            && !self.has_float_operand(&binop.left)
            && self.expr_is_unsigned_int(&binop.left)
        {
            let local = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func: RuntimeFn::IntShift,
                    args: vec![left, right, Operand::Const(IrValue::I64(2))],
                });
            return Ok(Operand::Local(local));
        }

        let ty = match binop.operator {
            BinaryOperator::Add
            | BinaryOperator::Sub
            | BinaryOperator::Mul
            | BinaryOperator::Div => {
                // 任一侧是 f64（小字面量或动态归一后的绑定）→ 结果走 F64 槽
                if dyn_math
                    || self.has_float_operand(&binop.left)
                    || self.has_float_operand(&binop.right)
                {
                    IrType::F64
                } else {
                    IrType::I64
                }
            }
            // `%` 与解释器 f64 rem 语义一致：任一侧是小数则结果按 F64 槽承载
            BinaryOperator::Mod => {
                if self.has_float_operand(&binop.left) || self.has_float_operand(&binop.right) {
                    IrType::F64
                } else {
                    IrType::I64
                }
            }
            // ** 与解释器 powf 语义一致：任一侧是小数则结果按 F64 槽承载
            BinaryOperator::Pow => {
                if dyn_math
                    || self.has_float_operand(&binop.left)
                    || self.has_float_operand(&binop.right)
                {
                    IrType::F64
                } else {
                    IrType::I64
                }
            }
            // 位运算在解释器里把操作数截断成 i64 再把结果 `as f64`：浮点侧
            // 参与时结果槽也是 F64，打印仍走 tag=2
            BinaryOperator::BitAnd
            | BinaryOperator::BitOr
            | BinaryOperator::Xor
            | BinaryOperator::Shl
            | BinaryOperator::Shr => {
                if self.has_float_operand(&binop.left) || self.has_float_operand(&binop.right) {
                    IrType::F64
                } else {
                    IrType::Bool
                }
            }
            _ => IrType::Bool,
        };

        let local = self.module.functions[func_idx].add_local(ty);

        let instr = match binop.operator {
            BinaryOperator::Add => Instruction::Add {
                dst: local,
                left,
                right,
            },
            BinaryOperator::Sub => Instruction::Sub {
                dst: local,
                left,
                right,
            },
            BinaryOperator::Mul => Instruction::Mul {
                dst: local,
                left,
                right,
            },
            BinaryOperator::Div => Instruction::Div {
                dst: local,
                left,
                right,
            },
            BinaryOperator::Mod => Instruction::Mod {
                dst: local,
                left,
                right,
            },
            BinaryOperator::Pow => Instruction::Pow {
                dst: local,
                left,
                right,
            },
            BinaryOperator::Eq => Instruction::Eq {
                dst: local,
                left,
                right,
            },
            BinaryOperator::Neq => Instruction::Neq {
                dst: local,
                left,
                right,
            },
            BinaryOperator::Lt => Instruction::Lt {
                dst: local,
                left,
                right,
            },
            BinaryOperator::Gt => Instruction::Gt {
                dst: local,
                left,
                right,
            },
            BinaryOperator::Lte => Instruction::Lte {
                dst: local,
                left,
                right,
            },
            BinaryOperator::Gte => Instruction::Gte {
                dst: local,
                left,
                right,
            },
            BinaryOperator::And => Instruction::And {
                dst: local,
                left,
                right,
            },
            BinaryOperator::Or => Instruction::Or {
                dst: local,
                left,
                right,
            },
            BinaryOperator::BitAnd => Instruction::BitAnd {
                dst: local,
                left,
                right,
            },
            BinaryOperator::BitOr => Instruction::BitOr {
                dst: local,
                left,
                right,
            },
            BinaryOperator::Xor => Instruction::Xor {
                dst: local,
                left,
                right,
            },
            BinaryOperator::Shl => Instruction::Shl {
                dst: local,
                left,
                right,
            },
            BinaryOperator::Shr => Instruction::Shr {
                dst: local,
                left,
                right,
            },
        };

        self.module.functions[func_idx].instructions.push(instr);
        Ok(Operand::Local(local))
    }

    pub(super) fn generate_unary(&mut self, unary: &UnaryOp) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let operand = self.generate_expression(&unary.operand)?;

        let ty = match unary.operator {
            UnaryOperator::Not => IrType::Bool,
            UnaryOperator::Neg => match self.operand_slot_type(&operand) {
                // 小数取负仍是 float（codegen 对 F64 操作数用 fneg）
                IrType::F64 => IrType::F64,
                _ => IrType::I64,
            },
            // `~` 对齐解释器 `(!(n as i64)) as f64`：浮点操作数结果落 F64 槽
            UnaryOperator::BitNot => {
                if self.has_float_operand(&unary.operand) {
                    IrType::F64
                } else {
                    IrType::I64
                }
            }
            // `move`/`copy` keep the operand's slot: a copied `float` stays an
            // F64 slot and a copied list/struct stays a pointer.
            UnaryOperator::Move | UnaryOperator::Copy => self.operand_slot_type(&operand),
        };

        // Unary minus on an integer is checked as well (`-i64::MIN` traps;
        // `-u64` narrows to `int`, like the interpreter).
        if matches!(unary.operator, UnaryOperator::Neg) && !matches!(ty, IrType::F64) {
            let unsigned = self.expr_is_unsigned_int(&unary.operand);
            let local = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func: RuntimeFn::IntNeg,
                    args: vec![operand, Operand::Const(IrValue::I64(i64::from(unsigned)))],
                });
            return Ok(Operand::Local(local));
        }

        // `copy` duplicates the value (spec `memory-and-runtime.md`,
        // "Ownership with `copy`"): lists, maps, sets, structs, enums, tuples,
        // and builders get independent storage, while identity-carrying values
        // keep their identity.
        if matches!(unary.operator, UnaryOperator::Copy) {
            // A float slot already holds the value; `copy` is an identity and
            // must not round-trip through the i64 ABI slot (that would either
            // numerically convert the bits or need an explicit bitcast).
            if matches!(ty, IrType::F64) {
                let local = self.module.functions[func_idx].add_local(ty);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::GetLocal {
                        dst: local,
                        src: match operand {
                            Operand::Local(id) => id,
                            other => {
                                self.module.functions[func_idx].instructions.push(
                                    Instruction::SetLocal {
                                        dst: local,
                                        src: other,
                                    },
                                );
                                return Ok(Operand::Local(local));
                            }
                        },
                    });
                return Ok(Operand::Local(local));
            }
            let local = self.module.functions[func_idx].add_local(ty);
            let (func, args) = if self.expr_is_string_builder(&unary.operand) {
                (RuntimeFn::StringBuilderCopy, vec![operand.clone()])
            } else {
                let tag = self.tag_operand_for(&unary.operand, &operand);
                (RuntimeFn::ValueCopy, vec![operand.clone(), tag])
            };
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func,
                    args,
                });
            return Ok(Operand::Local(local));
        }

        let local = self.module.functions[func_idx].add_local(ty);

        let instr = match unary.operator {
            UnaryOperator::Not => Instruction::Not {
                dst: local,
                operand,
            },
            UnaryOperator::Neg => Instruction::Neg {
                dst: local,
                operand,
            },
            UnaryOperator::BitNot => Instruction::BitNot {
                dst: local,
                operand,
            },
            UnaryOperator::Move | UnaryOperator::Copy => {
                // `move`/`copy` are alias copies at the IR level: materialize
                // the operand into a fresh local.
                Instruction::GetLocal {
                    dst: local,
                    src: match operand {
                        Operand::Local(id) => id,
                        _ => {
                            // Non-local operand (const/index): fall back to a
                            // no-op copy through SetLocal.
                            self.module.functions[func_idx].instructions.push(
                                Instruction::SetLocal {
                                    dst: local,
                                    src: operand,
                                },
                            );
                            return Ok(Operand::Local(local));
                        }
                    },
                }
            }
        };

        self.module.functions[func_idx].instructions.push(instr);
        Ok(Operand::Local(local))
    }

    /// 按形参表排列实参：全命名调用重排到声明序并为缺省的形参填默认
    /// 表达式；位置调用在尾部缺省处补齐。默认表达式在调用点求值（字面量
    /// 默认值的主用法），引用前置形参的默认值暂不支持。
    pub(super) fn arrange_call_args(
        &mut self,
        call: &Call,
        params: &[(String, Option<Expression>)],
        extern_call: bool,
    ) -> Result<Vec<Operand>, XuloError> {
        let all_named =
            !call.arguments.is_empty() && call.arguments.iter().all(|a| a.name.is_some());
        let mut args = Vec::new();
        // 省略的 extern 参数（只有 `Ptr?` 可省，语义层放行）绑定 null
        // （spec ffi.md Calling）；普通 `fn` 缺无默认参数仍是防御性报错。
        let missing = |pname: &str| -> Result<Operand, XuloError> {
            if extern_call {
                return Ok(Operand::Const(IrValue::Null));
            }
            Err(XuloError::new(
                ErrorKind::Semantic,
                format!(
                    "function `{}` is missing required argument `{pname}`",
                    call.callee
                ),
            ))
        };
        if all_named {
            for (pname, default) in params {
                if let Some(arg) = call
                    .arguments
                    .iter()
                    .find(|a| a.name.as_deref() == Some(pname.as_str()))
                {
                    args.push(self.generate_expression(&arg.value)?);
                } else if let Some(d) = default {
                    args.push(self.generate_expression(d)?);
                } else {
                    args.push(missing(pname)?);
                }
            }
        } else {
            for arg in &call.arguments {
                args.push(self.generate_expression(&arg.value)?);
            }
            for (pname, default) in &params[call.arguments.len().min(params.len())..] {
                if let Some(d) = default {
                    args.push(self.generate_expression(d)?);
                } else {
                    args.push(missing(pname)?);
                }
            }
        }
        Ok(args)
    }

    pub(super) fn generate_call(&mut self, call: &Call) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();

        // 多参数 print：逐参数 stringify 后用空格拼接（与解释器 native_print 一致）
        if call.object.is_none()
            && matches!(call.callee.as_str(), "print" | "println")
            && call.arguments.len() > 1
        {
            return self.generate_print_multi(call);
        }

        // 结构体构造：Point(x: 1) -> NewObject（类型名按模块解析）
        if call.object.is_none()
            && call.method.is_none()
            && let Some(fields) = self.structs.get(&self.resolve_type(&call.callee)).cloned()
        {
            return self.generate_struct_ctor(call, &fields);
        }

        // `Enum::Variant(...)` 构造（语义层已校验；内置 Result 与本模块
        // 自己声明的 enum 都在这里落地）
        if call.object.is_none()
            && let Some((enum_name, variant)) = call.callee.split_once("::")
            && self.enum_variant_index(enum_name, variant).is_some()
        {
            return self.generate_enum_construct(enum_name, variant, &call.arguments);
        }

        // `std::process::Command::shell/direct(...)` 构造纯规格对象。
        if call.object.is_none()
            && (call.callee.ends_with("Command::shell") || call.callee.ends_with("Command::direct"))
        {
            let method = if call.callee.ends_with("Command::shell") {
                RuntimeFn::ProcessCommandShell
            } else {
                RuntimeFn::ProcessCommandDirect
            };
            let mut args = Vec::new();
            for arg in &call.arguments {
                args.push(self.generate_expression(&arg.value)?);
            }
            let local = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func: method,
                    args,
                });
            return Ok(Operand::Local(local));
        }

        // `std::http` 的服务器与数据类型：`Request(...)` / `Response(...)`
        // 构造、`Response.text/json` 静态速记，以及 Request/Response/Server
        // 的方法。语义层已把它们解析为内建类型；这里落到带标签的运行时对象。
        if call.object.is_none() && (call.callee == "Request" || call.callee.ends_with("::Request"))
        {
            let mut args = Vec::with_capacity(8);
            for field in [
                "method",
                "path",
                "query",
                "form",
                "headers",
                "body",
                "version",
                "remote_addr",
            ] {
                match call
                    .arguments
                    .iter()
                    .find(|a| a.name.as_deref() == Some(field))
                {
                    Some(a) => args.push(self.generate_expression(&a.value)?),
                    None => args.push(Operand::Const(IrValue::Null)),
                }
            }
            let local = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func: RuntimeFn::HttpRequestNew,
                    args,
                });
            return Ok(Operand::Local(local));
        }
        if call.object.is_none()
            && (call.callee == "Response" || call.callee.ends_with("::Response"))
        {
            let mut args = Vec::with_capacity(3);
            for field in ["status", "headers", "body"] {
                match call
                    .arguments
                    .iter()
                    .find(|a| a.name.as_deref() == Some(field))
                {
                    Some(a) => args.push(self.generate_expression(&a.value)?),
                    None => args.push(Operand::Const(IrValue::Null)),
                }
            }
            let local = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func: RuntimeFn::HttpResponseNew,
                    args,
                });
            return Ok(Operand::Local(local));
        }
        if let Some(object) = &call.object {
            if dotted_expr_path(object)
                .is_some_and(|path| path == "Response" || path.ends_with("::Response"))
            {
                let method = call.method.as_deref().unwrap_or(&call.callee);
                if method == "text" {
                    let body = match call.arguments.first() {
                        Some(a) => self.generate_expression(&a.value)?,
                        None => Operand::Const(IrValue::Null),
                    };
                    let status = match call.arguments.get(1) {
                        Some(a) => self.generate_expression(&a.value)?,
                        None => Operand::Const(IrValue::I64(200)),
                    };
                    let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::RuntimeCall {
                            dst: Some(local),
                            func: RuntimeFn::HttpResponseText,
                            args: vec![body, status],
                        });
                    return Ok(Operand::Local(local));
                }
                if method == "json" {
                    let value = match call.arguments.first() {
                        Some(a) => self.generate_expression(&a.value)?,
                        None => Operand::Const(IrValue::Null),
                    };
                    let tag = call
                        .arguments
                        .first()
                        .map(|a| self.tag_operand_for(&a.value, &value))
                        .unwrap_or(Operand::Const(IrValue::I64(6)));
                    let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::RuntimeCall {
                            dst: Some(local),
                            func: RuntimeFn::HttpResponseJson,
                            args: vec![value, tag],
                        });
                    return Ok(Operand::Local(local));
                }
            }
            let method = call.method.as_deref().unwrap_or(&call.callee);
            let user_receiver = self
                .struct_name_of_expr(object)
                .is_some_and(|name| self.structs.contains_key(&name));
            // `std::process` 的 Command 规格与 Child 句柄方法。
            if !user_receiver && matches!(method, "input" | "cwd" | "env" | "timeout") {
                let receiver = self.generate_expression(object)?;
                let a = match call.arguments.first() {
                    Some(a) => self.generate_expression(&a.value)?,
                    None => Operand::Const(IrValue::Null),
                };
                let b = if method == "env" {
                    match call.arguments.get(1) {
                        Some(a) => self.generate_expression(&a.value)?,
                        None => Operand::Const(IrValue::Null),
                    }
                } else {
                    Operand::Const(IrValue::Null)
                };
                let ms = if method == "timeout" {
                    a.clone()
                } else {
                    Operand::Const(IrValue::F64(-1.0))
                };
                let which = match method {
                    "input" => 0,
                    "cwd" => 1,
                    "env" => 2,
                    _ => 3,
                };
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(local),
                        func: RuntimeFn::ProcessCommandRefine,
                        args: vec![receiver, Operand::Const(IrValue::I64(which)), a, b, ms],
                    });
                return Ok(Operand::Local(local));
            }
            if !user_receiver && matches!(method, "run" | "launch") {
                let receiver = self.generate_expression(object)?;
                let func = if method == "run" {
                    RuntimeFn::ProcessCommandRun
                } else {
                    RuntimeFn::ProcessCommandLaunch
                };
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(local),
                        func,
                        args: vec![receiver],
                    });
                return Ok(Operand::Local(local));
            }
            if !user_receiver && matches!(method, "pid" | "kill" | "wait") {
                let receiver = self.generate_expression(object)?;
                let func = match method {
                    "pid" => RuntimeFn::ProcessChildPid,
                    "kill" => RuntimeFn::ProcessChildKill,
                    _ => RuntimeFn::ProcessChildWait,
                };
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(local),
                        func,
                        args: vec![receiver],
                    });
                return Ok(Operand::Local(local));
            }
            // `header` is shared by Request and Response and reads the same
            // `headers` field in both; `query_value`/`form_value` are
            // Request-only. A user struct's own method wins.
            if !user_receiver && matches!(method, "header" | "query_value" | "form_value") {
                let receiver = self.generate_expression(object)?;
                let name = match call.arguments.first() {
                    Some(a) => self.generate_expression(&a.value)?,
                    None => Operand::Const(IrValue::Null),
                };
                let which = match method {
                    "header" => 0,
                    "query_value" => 1,
                    _ => 2,
                };
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(local),
                        func: RuntimeFn::HttpRequestLookup,
                        args: vec![receiver, name, Operand::Const(IrValue::I64(which))],
                    });
                return Ok(Operand::Local(local));
            }
            if !user_receiver && method == "serve" {
                let receiver = self.generate_expression(object)?;
                let handler = match call.arguments.first() {
                    Some(a) => self.generate_expression(&a.value)?,
                    None => Operand::Const(IrValue::Null),
                };
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(local),
                        func: RuntimeFn::HttpServerServe,
                        args: vec![receiver, handler],
                    });
                return Ok(Operand::Local(local));
            }
            if !user_receiver && method == "addr" {
                let receiver = self.generate_expression(object)?;
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(local),
                        func: RuntimeFn::HttpServerAddr,
                        args: vec![receiver],
                    });
                return Ok(Operand::Local(local));
            }
        }

        if call.object.is_none() && call.callee == "Map::from_entries" {
            let mut args = Vec::new();
            for arg in &call.arguments {
                args.push(self.generate_expression(&arg.value)?);
            }
            let local = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func: RuntimeFn::MapFromEntries,
                    args,
                });
            return Ok(Operand::Local(local));
        }

        if call.object.is_none() && call.callee == "Set::from_list" {
            let mut args = Vec::new();
            for arg in &call.arguments {
                args.push(self.generate_expression(&arg.value)?);
            }
            let local = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func: RuntimeFn::SetFromList,
                    args,
                });
            return Ok(Operand::Local(local));
        }

        // `Array::filled<T, const N: int>(value)`: the length is a compile-time
        // const argument, so lower it to a runtime fill of `N` copies.
        if call.object.is_none() && call.callee == "Array::filled" {
            let Some(length) = call.type_args.as_ref().and_then(|args| args.get(1)) else {
                return Err(XuloError::new(
                    ErrorKind::Semantic,
                    "`Array::filled` requires a const length argument".to_string(),
                ));
            };
            let Some(argument) = call.arguments.first() else {
                return Err(XuloError::new(
                    ErrorKind::Semantic,
                    "`Array::filled` expects exactly 1 argument".to_string(),
                ));
            };
            let value = self.generate_expression(&argument.value)?;
            let tag = self.tag_operand_for(&argument.value, &value);
            let length = self.generate_const_argument(length)?;
            let local = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func: RuntimeFn::ArrayFilled,
                    args: vec![value, tag, length],
                });
            return Ok(Operand::Local(local));
        }

        // `Task` combinators under the compiled async model: an async call
        // completes before it returns, so `Task::resolve` and `Task::all` are
        // the identity on the already-settled value, `Task::race` takes the
        // first argument (the first to run to completion), and `Task::timeout`
        // compares the value's measured runtime with the limit
        // (spec `concurrency.md`, "Task combinators").
        if call.object.is_none()
            && matches!(
                call.callee.as_str(),
                "Task::resolve" | "Task::all" | "Task::race"
            )
        {
            let Some(argument) = call.arguments.first() else {
                return Err(XuloError::new(
                    ErrorKind::Semantic,
                    format!("`{}` expects one argument", call.callee),
                ));
            };
            let value = self.generate_expression(&argument.value)?;
            if call.callee == "Task::race" {
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(local),
                        func: RuntimeFn::TaskRace,
                        args: vec![value],
                    });
                return Ok(Operand::Local(local));
            }
            return Ok(value);
        }

        if call.object.is_none() && call.callee == "Task::timeout" {
            let (Some(task), Some(limit)) = (call.arguments.first(), call.arguments.get(1)) else {
                return Err(XuloError::new(
                    ErrorKind::Semantic,
                    "`Task::timeout` expects a task and a millisecond count".to_string(),
                ));
            };
            let start = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(start),
                    func: RuntimeFn::Intrinsic("xulo_time_monotonic_nanos"),
                    args: Vec::new(),
                });
            let value = self.generate_expression(&task.value)?;
            let tag = self.tag_operand_for(&task.value, &value);
            let end = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(end),
                    func: RuntimeFn::Intrinsic("xulo_time_monotonic_nanos"),
                    args: Vec::new(),
                });
            let limit = self.generate_expression(&limit.value)?;
            let local = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func: RuntimeFn::TaskTimeout,
                    args: vec![
                        value,
                        tag,
                        Operand::Local(start),
                        Operand::Local(end),
                        limit,
                    ],
                });
            return Ok(Operand::Local(local));
        }

        let mut args = Vec::new();

        // 对于方法调用，先生成 receiver 并作为第一个参数
        if let Some(obj) = &call.object {
            let receiver = self.generate_expression(obj)?;
            args.push(receiver);
        }

        // 用户函数调用：命名参数按形参序重排，缺省参数在调用点补齐；
        // extern 导入用同一套排列（省略补 null，spec ffi.md Calling）
        if call.object.is_none()
            && let Some(params) = self
                .func_params
                .get(&self.resolve_callee_name(&call.callee))
                .cloned()
        {
            let extern_call = self
                .extern_map
                .contains_key(&self.resolve_callee_name(&call.callee));
            args.extend(self.arrange_call_args(call, &params, extern_call)?);
        } else {
            for arg in &call.arguments {
                args.push(self.generate_expression(&arg.value)?);
            }
        }

        // A call to a const-generic function supplies its const arguments as
        // hidden trailing parameters (compile-time values, spec
        // `types/generics.md`).
        if call.object.is_none() {
            let key = self.resolve_callee_name(&call.callee);
            if let Some(gps) = self.func_generic_params.get(&key).cloned() {
                for (i, p) in gps.iter().enumerate() {
                    if p.kind != xulo_core::ast::GenericParamKind::Const {
                        continue;
                    }
                    let Some(arg) = call.type_args.as_ref().and_then(|a| a.get(i)) else {
                        return Err(XuloError::new(
                            ErrorKind::Semantic,
                            format!(
                                "`{}` requires a written const argument for `{}`",
                                call.callee, p.name
                            ),
                        ));
                    };
                    args.push(self.generate_const_argument(arg)?);
                }
            }
        }

        // Trait 派发：语义层把 `Trait::method(recv)` 与 `str(v)` 标注成
        // `impl_{Trait}_{Type}_{method}`（泛型参数上的派发标注为 `dyn:`）。
        // AOT 把派发名静态解析到已注册的 impl 函数，位置与解释器 call()
        // 的前置分支一致：接收者就是实参序列的第一项。
        if let Some(impl_name) = &call.trait_impl {
            let Some(fid) = self.trait_impls.get(impl_name).copied().or_else(|| {
                self.module
                    .functions
                    .iter()
                    .position(|f| &f.name == impl_name)
                    .map(FuncId)
            }) else {
                if let Some(rest) = impl_name.strip_prefix("dyn:") {
                    return Err(XuloError::new(
                        ErrorKind::Semantic,
                        format!(
                            "AOT codegen does not support dynamic trait dispatch through a generic parameter (`{rest}`): the receiver's type is not static in a generic body"
                        ),
                    ));
                }
                return Err(XuloError::new(
                    ErrorKind::Semantic,
                    format!("undefined impl function `{impl_name}`"),
                ));
            };
            // `str(v)` / `${v}` / `v.to_string()` dispatch to `display` at the
            // default `Format`; a written `Display::display(v, fmt)` already
            // carries one (spec `types/traits.md`).
            if args.len() == 1
                && ((impl_name.starts_with("impl_Display_") && impl_name.ends_with("_display"))
                    || (impl_name.starts_with("impl_Debug_") && impl_name.ends_with("_debug")))
            {
                let format = self.default_format_operand();
                args.push(format);
            }
            let ret_type = self.module.functions[fid.0].return_type.clone();
            let local = self.module.functions[func_idx].add_local(ret_type);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Call {
                    dst: Some(local),
                    func: fid,
                    args,
                });
            return Ok(Operand::Local(local));
        }

        // str(字符串) 直通：模板 desugar 会对字符串也包一层 str()
        if call.object.is_none()
            && call.callee == "str"
            && args.len() == 1
            && self.determine_element_tag(&call.arguments[0].value) == 0
        {
            return Ok(args.swap_remove(0));
        }

        if call.object.is_some() && self.expr_is_map_receiver(&call.object) {
            let method = call.method.as_deref().unwrap_or(&call.callee);
            if matches!(
                method,
                "get" | "set" | "has" | "remove" | "keys" | "values" | "entries"
            ) {
                return self.generate_map_member(call, args);
            }
        }

        if call.object.is_some() && self.expr_is_set_receiver(&call.object) {
            let method = call.method.as_deref().unwrap_or(&call.callee);
            if matches!(method, "has" | "insert" | "remove") {
                return self.generate_set_member(call, args);
            }
        }

        // `std::math::` / `std::time::` / `std::fs::` / `std::io::` /
        // `std::os::` / `std::path::` / `std::net::` / `std::process::` /
        // `std::json::` 命名空间内建（语义层已按 builtin_member 校验成员
        // 存在；裸名调用已被 apply_std_calls 改写为限定形式）
        if call.object.is_none()
            && (call.callee.starts_with("std::math::")
                || call.callee.starts_with("std::time::")
                || call.callee.starts_with("std::fs::")
                || call.callee.starts_with("std::io::")
                || call.callee.starts_with("std::os::")
                || call.callee.starts_with("std::path::")
                || call.callee.starts_with("std::net::")
                || call.callee.starts_with("std::process::")
                || call.callee.starts_with("std::json::")
                || call.callee.starts_with("std::fmt::")
                || call.callee.starts_with("std::http::")
                || call.callee.starts_with("std::hint::"))
        {
            // `std::process::run`/`exec` 的可选 `input`/`timeout` 缺席时补
            // NULL 指针与 -1.0（FFI 判空 → 不接管 stdin / 不限时）——顺序
            // 补齐，缺 1 个只补尾参，缺 2 个两个都补
            if call.callee == "std::process::run" {
                if args.len() == 1 {
                    args.push(Operand::Const(IrValue::Null));
                }
                if args.len() == 2 {
                    args.push(Operand::Const(IrValue::F64(-1.0)));
                }
            }
            if call.callee == "std::process::exec" {
                if args.len() == 2 {
                    args.push(Operand::Const(IrValue::Null));
                }
                if args.len() == 3 {
                    args.push(Operand::Const(IrValue::F64(-1.0)));
                }
            }
            if call.callee == "std::process::launch" {
                if args.len() == 1 {
                    args.push(Operand::Const(IrValue::Null));
                }
                if args.len() == 2 {
                    args.push(Operand::Const(IrValue::F64(-1.0)));
                }
            }
            // `std::fs::temp_file([prefix])` / `temp_dir([prefix])` 的可选前缀
            // 与 `temp_file_in`/`temp_dir_in` 的可选第二参缺席时补 NULL 指针
            //（FFI 判空 → 无前缀），与 process::run 的可选 input 同一种补齐
            if matches!(
                call.callee.as_str(),
                "std::fs::temp_file" | "std::fs::temp_dir"
            ) && args.is_empty()
            {
                args.push(Operand::Const(IrValue::Null));
            }
            if matches!(
                call.callee.as_str(),
                "std::fs::temp_file_in" | "std::fs::temp_dir_in"
            ) && args.len() == 1
            {
                args.push(Operand::Const(IrValue::Null));
            }
            // `std::net` 的四个阻塞成员的可选 `timeout`（毫秒）缺席时补
            // -1.0（FFI 把负数/NaN 读作无预算），f64 形参不能收 NULL
            if matches!(
                call.callee.as_str(),
                "std::net::tcp_connect" | "std::net::tcp_accept"
            ) && args.len() == 1
            {
                args.push(Operand::Const(IrValue::F64(-1.0)));
            }
            if matches!(
                call.callee.as_str(),
                "std::net::sock_read" | "std::net::sock_write"
            ) && args.len() == 2
            {
                args.push(Operand::Const(IrValue::F64(-1.0)));
            }
            // `std::time::from_parts(year, month, day[, hour[, minute[,
            // second[, millisecond]]]])` 的四个可选时间字段缺席时补 0.0
            if call.callee == "std::time::from_parts" {
                while args.len() < 7 {
                    args.push(Operand::Const(IrValue::F64(0.0)));
                }
            }
            // `std::http::fetch`/`request` 的可选 `headers`/`body`/`timeout`
            // 缺席时补 NULL 指针与 -1.0（FFI 判空 → 无头/无体/不限时）
            if call.callee == "std::http::fetch" && args.len() == 1 {
                args.push(Operand::Const(IrValue::F64(-1.0)));
            }
            if call.callee == "std::http::request" {
                if args.len() == 2 {
                    args.push(Operand::Const(IrValue::Null));
                }
                if args.len() == 3 {
                    args.push(Operand::Const(IrValue::Null));
                }
                if args.len() == 4 {
                    args.push(Operand::Const(IrValue::F64(-1.0)));
                }
            }
            // `std::fmt::format(template[, args])` 的可选参数表缺席时补
            // NULL 指针（FFI 判空 → 空表）
            if call.callee == "std::fmt::format" {
                if args.len() == 1 {
                    args.push(Operand::Const(IrValue::Null));
                }
                let elements = call.arguments.get(1).and_then(|a| match &a.value {
                    Expression::Literal {
                        value: Literal::List(elems),
                        ..
                    } => Some(elems.as_slice()),
                    _ => None,
                });
                let meta = self.fmt_meta_operand(elements);
                args.push(meta);
            }
            // `std::json::stringify(value)` 需要值的运行时标签一并传给
            // FFI（与 print/str 同一套 dyn 判定：match 绑定走伴随标签槽，
            // 其余按源表达式静态推断）
            if call.callee == "std::json::stringify" && (1..=2).contains(&args.len()) {
                let tag = {
                    let arg_expr = &call.arguments[0].value;
                    self.tag_operand_for(arg_expr, &args[0])
                };
                let indent = if args.len() == 2 { args.pop() } else { None };
                args.push(tag);
                match indent {
                    Some(indent) => args.push(indent),
                    None => args.push(Operand::Const(IrValue::I64(-1))),
                }
            }
            if call.callee == "std::hint::black_box" {
                return self.generate_black_box_call(call, args);
            }
            return self.generate_intrinsic_call(&call.callee, &args);
        }

        // `RegExp::compile(pattern[, flags])` -> Result<RegExp, RegexError>
        if call.object.is_none() && call.callee == "RegExp::compile" {
            if args.len() == 1 {
                let empty = self.module.functions[func_idx].add_local(IrType::Pointer);
                let empty_idx = self.module.add_string(String::new());
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::Const {
                        dst: empty,
                        value: IrValue::String(empty_idx),
                    });
                args.push(Operand::Local(empty));
            }
            let local = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func: RuntimeFn::RegexCompile,
                    args,
                });
            return Ok(Operand::Local(local));
        }

        // 检查是否是内置函数或内置方法
        // receiver 是用户结构体时不抢方法名（用户 impl 方法优先）；内置
        // Named 类型（StringBuilder 等）不是结构体，仍需走内建 lowering。
        let receiver_is_struct = call.object.as_ref().is_some_and(|o| {
            self.struct_name_of_expr(o)
                .is_some_and(|name| self.structs.contains_key(&name))
        });
        let receiver_is_builder = call
            .object
            .as_ref()
            .is_some_and(|o| self.expr_is_string_builder(o));
        let mut runtime_fn = if call.object.is_some() {
            // 方法调用：根据方法名匹配
            match call.method.as_deref().unwrap_or(&call.callee) {
                "push" => {
                    // push 需要类型标签作为第三个参数
                    if let Some(arg) = call.arguments.first() {
                        let tag = self.determine_element_tag(&arg.value);
                        args.push(Operand::Const(IrValue::I64(tag)));
                    } else {
                        args.push(Operand::Const(IrValue::I64(6))); // null
                    }
                    Some(RuntimeFn::ArrayPush)
                }
                // RegExp 实例查询（receiver 是 RegExp 指针）
                "test" if !receiver_is_struct => Some(RuntimeFn::RegexTest),
                "find" if !receiver_is_struct => Some(RuntimeFn::RegexFind),
                "find_all" if !receiver_is_struct => Some(RuntimeFn::RegexFindAll),
                "group" if !receiver_is_struct => Some(RuntimeFn::RegexMatchGroup),
                // 字符串替换：模板 or 替换闭包（JIT 闭包经固定入口协议回调）
                "replace" => {
                    let use_closure = call.arguments.get(1).is_some_and(|a| {
                        matches!(
                            &a.value,
                            Expression::CallValue(_) | Expression::FnExpr(_) | Expression::Arrow(_)
                        ) || matches!(
                            self.infer_source_type_from_expr(&a.value),
                            Some(Type::FnSig { .. })
                        )
                    });
                    // receiver 是 input；运行时签名 (re, input, 第三参)
                    if args.len() >= 2 {
                        args.swap(0, 1);
                    }
                    if use_closure {
                        Some(RuntimeFn::RegexReplaceWith)
                    } else {
                        Some(RuntimeFn::RegexReplace)
                    }
                }
                "to_uppercase" if !receiver_is_struct => Some(RuntimeFn::StringToUppercase),
                "to_lowercase" if !receiver_is_struct => Some(RuntimeFn::StringToLowercase),
                "trim" if !receiver_is_struct => Some(RuntimeFn::StringTrim),
                // StringBuilder 实例方法：只按静态类型派发，避免与列表/
                // 用户方法的同名成员冲突。
                "append" if receiver_is_builder => Some(RuntimeFn::StringBuilderAppend),
                "clear" if receiver_is_builder => Some(RuntimeFn::StringBuilderClear),
                "build" if receiver_is_builder => Some(RuntimeFn::StringBuilderBuild),
                "starts_with" if !receiver_is_struct => Some(RuntimeFn::StringStartsWith),
                "ends_with" if !receiver_is_struct => Some(RuntimeFn::StringEndsWith),
                // `split(sep)`: a string separator, or a compiled `RegExp`
                // (payload tag 9) — the runtime gets the tag to tell them apart.
                "split" if !receiver_is_struct => {
                    let tag = match (call.arguments.first(), args.last()) {
                        (Some(arg), Some(value)) => self.tag_operand_for(&arg.value, value),
                        _ => Operand::Const(IrValue::I64(0)),
                    };
                    args.push(tag);
                    Some(RuntimeFn::StringSplit)
                }
                "pad_start" if !receiver_is_struct => Some(RuntimeFn::StringPadStart),
                "pad_end" if !receiver_is_struct => Some(RuntimeFn::StringPadEnd),
                "repeat" if !receiver_is_struct => Some(RuntimeFn::StringRepeat),
                // Built-in `string` members (spec `types/primitive-types.md`).
                "code_at" if !receiver_is_struct => Some(RuntimeFn::StringCodeAt),
                "bytes" if !receiver_is_struct => Some(RuntimeFn::StringBytes),
                "slice" if !receiver_is_struct && !self.expr_is_list_receiver(&call.object) => {
                    Some(RuntimeFn::StringSlice)
                }
                // `contains`/`length` 在列表与字符串上重名：列表接收者保持
                // 原路径（这些成员尚未 lowering，按用户函数解析报错）
                "contains" if !receiver_is_struct && !self.expr_is_list_receiver(&call.object) => {
                    Some(RuntimeFn::StringContains)
                }
                "length" if !receiver_is_struct && !self.expr_is_list_receiver(&call.object) => {
                    Some(RuntimeFn::StringLength)
                }
                _ => None,
            }
        } else {
            // 普通函数调用
            match call.callee.as_str() {
                "StringBuilder::new" => Some(RuntimeFn::StringBuilderNew),
                "print" | "println" => {
                    // 使用 PrintValue 带类型标签
                    if args.is_empty() {
                        // print() 无参数: 传 (0, 6) 打印空行
                        args.push(Operand::Const(IrValue::I64(0)));
                        args.push(Operand::Const(IrValue::I64(6)));
                    } else {
                        // 从原始 AST 表达式推断类型标签（而非生成后的 operand）；
                        // match 绑定用伴随的运行时标签槽。用户类型的
                        // `Display`/`Debug` 先于输出表咨询（spec
                        // `builtins/intrinsic-functions.md`, "Output"）。
                        let arg_expr = &call.arguments[0].value;
                        if self.expr_is_string_builder(arg_expr) {
                            // `print(sb)` renders the builder's current text,
                            // exactly the `string` row (spec
                            // `builtins/intrinsic-functions.md`).
                            let func_idx = self.current_func.unwrap();
                            let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                            self.module.functions[func_idx].instructions.push(
                                Instruction::RuntimeCall {
                                    dst: Some(local),
                                    func: RuntimeFn::StringBuilderBuild,
                                    args: vec![args[0].clone()],
                                },
                            );
                            args = vec![Operand::Local(local), Operand::Const(IrValue::I64(0))];
                        } else if let Some(rendered) =
                            self.user_print_operand(arg_expr, args[0].clone())
                        {
                            args = vec![rendered, Operand::Const(IrValue::I64(0))];
                        } else {
                            let tag_op = self.tag_operand_for(arg_expr, &args[0]);
                            args.push(tag_op);
                        }
                    }
                    Some(RuntimeFn::PrintValue)
                }
                "panic" => Some(RuntimeFn::Panic),
                "str" => {
                    // 带标签转换：整数 0 不能被当成 null（旧 xulo_to_string 是 tag-blind 的）
                    if args.is_empty() {
                        args.push(Operand::Const(IrValue::I64(0)));
                    }
                    let tag_op = if let Some(arg) = call.arguments.first() {
                        self.tag_operand_for(&arg.value, &args[0])
                    } else {
                        Operand::Const(IrValue::I64(6)) // null
                    };
                    args.push(tag_op);
                    Some(RuntimeFn::Stringify)
                }
                _ => None,
            }
        };

        // 字面量 append 的字节长度和标量个数在编译期已知，交给运行时快路径，
        // 避免每次追加都扫描 NUL 并重新统计 UTF-8 标量。
        if runtime_fn == Some(RuntimeFn::StringBuilderAppend)
            && call.arguments.len() == 1
            && let Expression::Literal {
                value: Literal::String(text),
                ..
            } = &call.arguments[0].value
        {
            args.push(Operand::Const(IrValue::I64(text.len() as i64)));
            args.push(Operand::Const(IrValue::I64(text.chars().count() as i64)));
            runtime_fn = Some(RuntimeFn::StringBuilderAppendKnown);
        }

        if let Some(rf) = runtime_fn {
            // PrintValue 和 Print 不返回值，不需要 dst
            let no_return = matches!(
                rf,
                RuntimeFn::PrintValue
                    | RuntimeFn::Print
                    | RuntimeFn::Panic
                    | RuntimeFn::ArrayPush
                    | RuntimeFn::ArraySet
                    | RuntimeFn::ObjectSet
            );
            if no_return {
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: None,
                        func: rf,
                        args,
                    });
                // 返回一个虚拟值（不会被使用）
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                Ok(Operand::Local(local))
            } else {
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(local),
                        func: rf,
                        args,
                    });
                Ok(Operand::Local(local))
            }
        } else {
            // 局部变量持有函数值（闭包/箭头/内嵌 fn 绑定/函数 thunk）→
            // 间接调用，优先于模块函数解析（允许局部遮蔽）。
            // 装箱名（如自递归内嵌 fn）的本地槽是 cell 指针，先解包
            if call.object.is_none()
                && let Some(&local) = self.locals.get(&call.callee)
            {
                let callee_op = if self.boxed_names.contains(&call.callee) {
                    let cell_local = self.module.functions[func_idx].add_local(IrType::Pointer);
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::RuntimeCall {
                            dst: Some(cell_local),
                            func: RuntimeFn::CellGet,
                            args: vec![Operand::Local(local)],
                        });
                    Operand::Local(cell_local)
                } else {
                    Operand::Local(local)
                };
                return self.generate_call_indirect(callee_op, args);
            }
            // FFI 导入（extern 块声明）：经 ExternCall 按 C ABI 走符号解析
            // （JIT finalize 时 dlsym，AOT 留给系统链接器）。导入不是函数值
            // （语义 E0903 拦绑定），局部绑定已在上面先遮蔽。
            if call.object.is_none()
                && let Some(&ex_idx) = self.extern_map.get(&self.resolve_callee_name(&call.callee))
            {
                let (arity, ret) = {
                    let decl = &self.module.externs[ex_idx];
                    (decl.params.len(), decl.return_type)
                };
                if arity != args.len() {
                    return Err(XuloError::new(
                        ErrorKind::Semantic,
                        format!(
                            "extern function expects {} arguments, got {}",
                            arity,
                            args.len()
                        ),
                    ));
                }
                let dst = ret.map(|p| {
                    let ty = if matches!(p, IrAbiParam::F32 | IrAbiParam::F64) {
                        IrType::F64
                    } else {
                        IrType::Pointer
                    };
                    self.module.functions[func_idx].add_local(ty)
                });
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::ExternCall {
                        dst,
                        extern_idx: ex_idx,
                        args,
                    });
                // unit 返回给调用点一个虚拟槽（与 RuntimeCall 的 no-return 一致）
                let result = dst
                    .unwrap_or_else(|| self.module.functions[func_idx].add_local(IrType::Pointer));
                return Ok(Operand::Local(result));
            }
            // 用户定义函数 / impl 方法
            let callee_idx = self.resolve_callee(call).ok_or_else(|| {
                let name = if call.object.is_some() {
                    call.method.as_deref().unwrap_or(&call.callee).to_string()
                } else {
                    call.callee.clone()
                };
                XuloError::new(ErrorKind::Semantic, format!("undefined function: {}", name))
            })?;
            let ret_type = self.module.functions[callee_idx].return_type.clone();
            let local = self.module.functions[func_idx].add_local(ret_type);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Call {
                    dst: Some(local),
                    func: FuncId(callee_idx),
                    args,
                });
            Ok(Operand::Local(local))
        }
    }

    /// 经函数值间接调用：实参超过 8 个无法塞进闭包入口协议
    pub(super) fn generate_call_indirect(
        &mut self,
        callee: Operand,
        args: Vec<Operand>,
    ) -> Result<Operand, XuloError> {
        if args.len() > 8 {
            return Err(XuloError::new(
                ErrorKind::Semantic,
                format!(
                    "calling a function value supports at most 8 arguments, got {}",
                    args.len()
                ),
            ));
        }
        let func_idx = self.current_func.unwrap();
        let local = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::CallIndirect {
                dst: Some(local),
                callee,
                args,
            });
        Ok(Operand::Local(local))
    }

    /// 解析被调函数：方法分派优先按 `Type::method`，退化到方法名唯一分派与顶层函数
    pub(super) fn resolve_callee(&self, call: &Call) -> Option<usize> {
        if let Some(obj) = &call.object {
            let method = call.method.as_deref().unwrap_or(&call.callee);
            // 1) receiver 静态类型 -> Type::method
            if let Some(recv_ty) = self.struct_name_of_expr(obj)
                && let Some(&fid) = self.methods.get(&format!("{}::{}", recv_ty, method))
            {
                return Some(fid.0);
            }
            // 2) 方法名唯一分派
            if let Some(types) = self.method_types.get(method)
                && types.len() == 1
                && let Some(&fid) = self.methods.get(&format!("{}::{}", types[0], method))
            {
                return Some(fid.0);
            }
            // 3) 裸名顶层函数
            self.module.functions.iter().position(|f| f.name == method)
        } else {
            let name = self.resolve_callee_name(&call.callee);
            self.module.functions.iter().position(|f| f.name == name)
        }
    }

    /// `std::hint::black_box`：把实参交给运行时的优化屏障再返回。float 槽走
    /// f64 入口，其余槽（整数/字符串/列表/对象/枚举）走 i64 入口；结果槽
    /// 与实参同域，后续算术/打印沿用原标签。
    pub(super) fn generate_black_box_call(
        &mut self,
        call: &Call,
        mut args: Vec<Operand>,
    ) -> Result<Operand, XuloError> {
        let float = call
            .arguments
            .first()
            .is_some_and(|a| self.has_float_operand(&a.value));
        let (sym, ty) = if float {
            ("xulo_hint_black_box_f64", IrType::F64)
        } else {
            ("xulo_hint_black_box", IrType::I64)
        };
        let func_idx = self.current_func.unwrap();
        let local = self.module.functions[func_idx].add_local(ty);
        let arg = args.remove(0);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(local),
                func: RuntimeFn::Intrinsic(sym),
                args: vec![arg],
            });
        Ok(Operand::Local(local))
    }

    /// Math::/Time:: 内建调用：RuntimeCall 落到对应运行时符号
    pub(super) fn generate_intrinsic_call(
        &mut self,
        callee: &str,
        args: &[Operand],
    ) -> Result<Operand, XuloError> {
        let Some((sym, ty, _tag)) = Self::intrinsic_lowering(callee) else {
            // 未知成员语义层已拦（E0212/E0003）；此处兜底防御
            return Err(XuloError::new(
                ErrorKind::Semantic,
                format!("undefined function: {}", callee),
            ));
        };
        let func_idx = self.current_func.unwrap();
        let local = self.module.functions[func_idx].add_local(ty);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(local),
                func: RuntimeFn::Intrinsic(sym),
                args: args.to_vec(),
            });
        Ok(Operand::Local(local))
    }

    /// 多参数 print：每个参数 xulo_stringify(value, tag) 后用 " " 折叠拼接，最终 PrintValue
    pub(super) fn generate_print_multi(&mut self, call: &Call) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();

        // 分隔符 " " 先物化进 local（Const String 不能直接作 RuntimeCall 实参）
        let space_local = self.module.functions[func_idx].add_local(IrType::Pointer);
        let space_idx = self.module.add_string(" ".to_string());
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Const {
                dst: space_local,
                value: IrValue::String(space_idx),
            });

        let mut joined: Option<Operand> = None;
        for arg in &call.arguments {
            let v = self.generate_expression(&arg.value)?;
            let next = if let Some(rendered) = self.user_print_operand(&arg.value, v.clone()) {
                rendered
            } else {
                let tag = self.tag_operand_for(&arg.value, &v);
                let s_local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(s_local),
                        func: RuntimeFn::Stringify,
                        args: vec![v, tag],
                    });
                Operand::Local(s_local)
            };
            joined = Some(match joined {
                None => next,
                Some(prev) => {
                    let cat = self.module.functions[func_idx].add_local(IrType::Pointer);
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::RuntimeCall {
                            dst: Some(cat),
                            func: RuntimeFn::StringConcat,
                            args: vec![prev, Operand::Local(space_local)],
                        });
                    let cat2 = self.module.functions[func_idx].add_local(IrType::Pointer);
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::RuntimeCall {
                            dst: Some(cat2),
                            func: RuntimeFn::StringConcat,
                            args: vec![Operand::Local(cat), next],
                        });
                    Operand::Local(cat2)
                }
            });
        }

        let acc = joined.unwrap_or(Operand::Const(IrValue::String(space_idx)));
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: None,
                func: RuntimeFn::PrintValue,
                args: vec![acc, Operand::Const(IrValue::I64(0))],
            });
        let local = self.module.functions[func_idx].add_local(IrType::Pointer);
        Ok(Operand::Local(local))
    }

    /// `a ?? b`：a 非 null 取 a，否则取 b（null 在运行时表示为 0）
    pub(super) fn generate_nullish(&mut self, nullish: &NullishExpr) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let left = self.generate_expression(&nullish.left)?;
        let right = self.generate_expression(&nullish.right)?;

        let keep_label = self.module.functions[func_idx].add_label();
        let alt_label = self.module.functions[func_idx].add_label();
        let end_label = self.module.functions[func_idx].add_label();
        let cond = self.module.functions[func_idx].add_local(IrType::Bool);
        let result = self.module.functions[func_idx].add_local(IrType::Pointer);

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Neq {
                dst: cond,
                left: left.clone(),
                right: Operand::Const(IrValue::Null),
            });
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Branch {
                condition: Operand::Local(cond),
                then_label: keep_label,
                else_label: alt_label,
            });

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(keep_label));
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::SetLocal {
                dst: result,
                src: left,
            });
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Jump(end_label));

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(alt_label));
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::SetLocal {
                dst: result,
                src: right,
            });
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Jump(end_label));

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(end_label));
        Ok(Operand::Local(result))
    }

    /// `cond ? then : else` lowers like a two-arm `if` expression; the else
    /// operand is itself an expression, so right-nested ternaries recurse.
    pub(super) fn generate_ternary(&mut self, ternary: &TernaryExpr) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let condition = self.generate_expression(&ternary.condition)?;
        let then_label = self.module.functions[func_idx].add_label();
        let else_label = self.module.functions[func_idx].add_label();
        let end_label = self.module.functions[func_idx].add_label();
        let result = self.module.functions[func_idx].add_local(IrType::Pointer);

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
        let then_value = self.generate_expression(&ternary.then_value)?;
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::SetLocal {
                dst: result,
                src: then_value,
            });
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Jump(end_label));

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(else_label));
        let else_value = self.generate_expression(&ternary.else_value)?;
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::SetLocal {
                dst: result,
                src: else_value,
            });
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Jump(end_label));

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(end_label));
        Ok(Operand::Local(result))
    }

    pub(super) fn generate_if_expr(&mut self, if_expr: &IfExpr) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let condition = self.generate_expression(&if_expr.condition)?;

        let then_label = self.module.functions[func_idx].add_label();
        let else_label = self.module.functions[func_idx].add_label();
        let end_label = self.module.functions[func_idx].add_label();
        let result_local = self.module.functions[func_idx].add_local(IrType::Pointer);

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
        let then_falls_through = self.generate_if_branch(&if_expr.then_branch, result_local)?;
        if then_falls_through {
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Jump(end_label));
        }

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(else_label));
        let else_falls_through = match &if_expr.else_branch {
            Some(branch) => self.generate_if_branch(branch, result_local)?,
            None => true,
        };
        if else_falls_through {
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Jump(end_label));
        }
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(end_label));

        Ok(Operand::Local(result_local))
    }

    /// Emit one `if` branch. Every statement is lowered in order (a `return`
    /// in the branch must run, not be skipped in favour of a tail
    /// expression); a trailing expression becomes the branch's value.
    /// Returns whether control falls through to the join label.
    fn generate_if_branch(
        &mut self,
        block: &Block,
        result_local: LocalId,
    ) -> Result<bool, XuloError> {
        let func_idx = self.current_func.unwrap();
        let last = block.statements.len().checked_sub(1);
        for (index, statement) in block.statements.iter().enumerate() {
            if Some(index) == last
                && let Statement::Expr(expr_stmt) = statement
            {
                let value = self.generate_expression(&expr_stmt.expr)?;
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::SetLocal {
                        dst: result_local,
                        src: value,
                    });
                return Ok(true);
            }
            self.generate_statement(statement)?;
        }
        Ok(!matches!(
            block.statements.last(),
            Some(Statement::Return(_) | Statement::Break | Statement::Continue)
        ))
    }

    /// `match` 表达式：按变体 tag 顺序测试，payload 槽摘出值与运行时标签，
    /// 分支结果写入统一的结果槽（spec control-flow.md；AOT 目前支持枚举
    /// 模式、通配与绑定，字面量/范围等模式语义层已另行检查）
    pub(super) fn generate_match(&mut self, m: &MatchExpr) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let scrut = self.generate_expression(&m.value)?;
        let scrut_local = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::SetLocal {
                dst: scrut_local,
                src: scrut,
            });
        let scrut_op = Operand::Local(scrut_local);

        // 字面量 pattern 需要 scrutinee 的标签：match 绑定用伴随槽，其余静态
        // 推断（-1 表示运行时检测）
        let scrut_tag_op = if let Expression::Identifier { name, .. } = &m.value
            && let Some(tag_op) = self.dyn_tag_of.get(name).cloned()
        {
            tag_op
        } else {
            Operand::Const(IrValue::I64(self.determine_element_tag(&m.value)))
        };

        let result_local = self.module.functions[func_idx].add_local(IrType::Pointer);
        let end_label = self.module.functions[func_idx].add_label();
        // 最后一个枚举臂的 next_label 落空，需要补一条跳到 end
        let mut dangling_next = false;

        for arm in &m.arms {
            if arm.guard.is_some() {
                return Err(XuloError::new(
                    ErrorKind::Semantic,
                    "AOT codegen does not support match guards yet".to_string(),
                ));
            }
            let body_label = self.module.functions[func_idx].add_label();
            let next_label = self.module.functions[func_idx].add_label();
            match &arm.pattern {
                MatchPattern::Enum(r) => {
                    self.emit_enum_tag_branch(
                        func_idx,
                        scrut_op.clone(),
                        &r.enum_name,
                        &r.variant,
                        body_label,
                        next_label,
                    )?;
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Label(body_label));
                    self.generate_match_arm_body(arm, result_local)?;
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Jump(end_label));
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Label(next_label));
                    dangling_next = true;
                }
                MatchPattern::EnumPayload {
                    enum_name,
                    variant,
                    bindings,
                    ..
                } => {
                    self.emit_enum_tag_branch(
                        func_idx,
                        scrut_op.clone(),
                        enum_name,
                        variant,
                        body_label,
                        next_label,
                    )?;
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Label(body_label));
                    for (i, bind) in bindings.iter().enumerate() {
                        if bind == "_" {
                            continue;
                        }
                        let raw = self.module.functions[func_idx].add_local(IrType::Pointer);
                        self.module.functions[func_idx].instructions.push(
                            Instruction::RuntimeCall {
                                dst: Some(raw),
                                func: RuntimeFn::EnumGet,
                                args: vec![
                                    scrut_op.clone(),
                                    Operand::Const(IrValue::I64(i as i64)),
                                ],
                            },
                        );
                        let tag = self.module.functions[func_idx].add_local(IrType::I64);
                        self.module.functions[func_idx].instructions.push(
                            Instruction::RuntimeCall {
                                dst: Some(tag),
                                func: RuntimeFn::EnumGetTag,
                                args: vec![
                                    scrut_op.clone(),
                                    Operand::Const(IrValue::I64(i as i64)),
                                ],
                            },
                        );
                        if self.boxed_names.contains(bind) {
                            // 被嵌套字面量捕获的绑定：装箱（伴随标签不入
                            // cell，读出时按 -1 运行时检测）
                            let cell = self.module.functions[func_idx].add_local(IrType::Pointer);
                            self.module.functions[func_idx].instructions.push(
                                Instruction::RuntimeCall {
                                    dst: Some(cell),
                                    func: RuntimeFn::AllocCell,
                                    args: vec![Operand::Local(raw)],
                                },
                            );
                            self.locals.insert(bind.clone(), cell);
                            self.boxed_logical.insert(bind.clone(), IrType::Pointer);
                        } else {
                            self.locals.insert(bind.clone(), raw);
                        }
                        self.dyn_tag_of.insert(bind.clone(), Operand::Local(tag));
                        // `read_dir`/`walk`/`process::args` 的 Ok 载荷是
                        // List<string>：绑定注册元素类型，`.len`/索引才会走
                        // 数组 lowering（否则 `.len` 误入 StringSize）
                        if matches!(
                            &m.value,
                            Expression::Call(call)
                                if call.object.is_none()
                                    && matches!(
                                        call.callee.as_str(),
                                        "std::fs::read_dir"
                                            | "std::fs::walk"
                                            | "std::process::args"
                                    )
                        ) {
                            self.source_types
                                .insert(bind.clone(), Type::List(Box::new(Type::String)));
                        } else if matches!(
                            &m.value,
                            Expression::Call(call)
                                if call.object.is_none()
                                    && matches!(
                                        call.callee.as_str(),
                                        "std::process::run" | "std::process::exec"
                                    )
                        ) {
                            // `run`/`exec` 的 Ok 载荷是 `{stdout, stderr, code}`：
                            // 字段类型必须静态可知，否则 `run.stdout +
                            // run.stderr` 会因两侧都不像 string 而降成整数
                            // 加法，指针相加后拼接即野指针。
                            self.source_types.insert(
                                bind.clone(),
                                Type::ObjectType(vec![
                                    ("stdout".to_string(), Type::String),
                                    ("stderr".to_string(), Type::String),
                                    ("code".to_string(), Type::Number),
                                ]),
                            );
                        } else if matches!(
                            &m.value,
                            Expression::Call(call)
                                if call.object.is_none()
                                    && call.callee == "std::fs::read_bytes"
                        ) {
                            self.source_types.insert(
                                bind.clone(),
                                Type::List(Box::new(Type::Named("u8".to_string(), Vec::new()))),
                            );
                        } else if matches!(
                            &m.value,
                            Expression::Call(call)
                                if call.object.is_none()
                                    && call.callee == "std::http::listen"
                        ) {
                            self.source_types.insert(
                                bind.clone(),
                                Type::Named("Server".to_string(), Vec::new()),
                            );
                        } else if matches!(
                            &m.value,
                            Expression::Call(call)
                                if call.object.is_none()
                                    && matches!(
                                        call.callee.as_str(),
                                        "std::http::fetch"
                                            | "std::http::request"
                                            | "std::http::Response::json"
                                    )
                        ) {
                            self.source_types.insert(
                                bind.clone(),
                                Type::Named("Response".to_string(), Vec::new()),
                            );
                        }
                    }
                    self.generate_match_arm_body(arm, result_local)?;
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Jump(end_label));
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Label(next_label));
                    dangling_next = true;
                }
                // 结构体模式 `P(x, y)`：结构体在运行时就是普通对象、没有类型
                // 标签，语义层已把 scrutinee 钉在该 struct 类型上，所以这个臂
                // 直接匹配，按声明序取字段（spec control-flow.md "Patterns"）。
                MatchPattern::Struct { name, bindings, .. } => {
                    let fields = self.structs.get(name).cloned().unwrap_or_default();
                    // 没有类型测试：直接落到本臂（先给上一块补终止指令）
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Jump(body_label));
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Label(body_label));
                    for (i, sub) in bindings.iter().enumerate() {
                        let MatchPattern::Binding { name: bind, .. } = sub else {
                            continue;
                        };
                        let Some(field) = fields.get(i) else {
                            continue;
                        };
                        let key = self.module.functions[func_idx].add_local(IrType::Pointer);
                        let key_idx = self.module.add_string(field.name.clone());
                        self.module.functions[func_idx]
                            .instructions
                            .push(Instruction::Const {
                                dst: key,
                                value: IrValue::String(key_idx),
                            });
                        let raw = self.module.functions[func_idx].add_local(IrType::Pointer);
                        self.module.functions[func_idx].instructions.push(
                            Instruction::RuntimeCall {
                                dst: Some(raw),
                                func: RuntimeFn::ObjectGet,
                                args: vec![scrut_op.clone(), Operand::Local(key)],
                            },
                        );
                        let tag = self.module.functions[func_idx].add_local(IrType::I64);
                        self.module.functions[func_idx].instructions.push(
                            Instruction::RuntimeCall {
                                dst: Some(tag),
                                func: RuntimeFn::ObjectGetTag,
                                args: vec![scrut_op.clone(), Operand::Local(key)],
                            },
                        );
                        if self.boxed_names.contains(bind) {
                            let cell = self.module.functions[func_idx].add_local(IrType::Pointer);
                            self.module.functions[func_idx].instructions.push(
                                Instruction::RuntimeCall {
                                    dst: Some(cell),
                                    func: RuntimeFn::AllocCell,
                                    args: vec![Operand::Local(raw)],
                                },
                            );
                            self.locals.insert(bind.clone(), cell);
                            self.boxed_logical.insert(bind.clone(), IrType::Pointer);
                        } else {
                            self.locals.insert(bind.clone(), raw);
                        }
                        self.dyn_tag_of.insert(bind.clone(), Operand::Local(tag));
                        self.source_types.insert(bind.clone(), field.type_.clone());
                    }
                    self.generate_match_arm_body(arm, result_local)?;
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Jump(end_label));
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Label(next_label));
                    dangling_next = true;
                }
                MatchPattern::Literal(lit) => {
                    let (lit_val, lit_tag) = self.literal_pattern_operand(lit)?;
                    let cmp = self.module.functions[func_idx].add_local(IrType::I64);
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::RuntimeCall {
                            dst: Some(cmp),
                            func: RuntimeFn::LiteralMatch,
                            args: vec![
                                scrut_op.clone(),
                                scrut_tag_op.clone(),
                                lit_val,
                                Operand::Const(IrValue::I64(lit_tag)),
                            ],
                        });
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Branch {
                            condition: Operand::Local(cmp),
                            then_label: body_label,
                            else_label: next_label,
                        });
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Label(body_label));
                    self.generate_match_arm_body(arm, result_local)?;
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Jump(end_label));
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Label(next_label));
                    dangling_next = true;
                }
                MatchPattern::Wildcard => {
                    self.generate_match_arm_body(arm, result_local)?;
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Jump(end_label));
                    // next 落点留空块：后续臂或收尾跳转都从这里接续
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Label(next_label));
                    dangling_next = true;
                }
                MatchPattern::Binding { name, .. } => {
                    if self.boxed_names.contains(name) {
                        let cell = self.module.functions[func_idx].add_local(IrType::Pointer);
                        self.module.functions[func_idx].instructions.push(
                            Instruction::RuntimeCall {
                                dst: Some(cell),
                                func: RuntimeFn::AllocCell,
                                args: vec![scrut_op.clone()],
                            },
                        );
                        self.locals.insert(name.clone(), cell);
                        self.boxed_logical.insert(name.clone(), IrType::Pointer);
                    } else {
                        self.locals.insert(name.clone(), scrut_local);
                    }
                    // 标签取 scrutinee 的伴随标签（字面量/标识符静态推断，或
                    // 标识符绑定时的运行时槽），而非硬编码 enum 标签
                    self.dyn_tag_of.insert(name.clone(), scrut_tag_op.clone());
                    self.generate_match_arm_body(arm, result_local)?;
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Jump(end_label));
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Label(next_label));
                    dangling_next = true;
                }
                _ => {
                    return Err(XuloError::new(
                        ErrorKind::Semantic,
                        "AOT codegen does not support range, struct, type, or or-patterns in match yet"
                            .to_string(),
                    ));
                }
            }
        }

        if dangling_next {
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Jump(end_label));
        }
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(end_label));
        Ok(Operand::Local(result_local))
    }

    /// 字面量 pattern 的比对操作数：返回 (值操作数, 标签)。字符串先物化进
    /// 本地槽（Const String 不能直接作 RuntimeCall 实参），小数走 f64 位模式。
    pub(super) fn literal_pattern_operand(
        &mut self,
        lit: &Literal,
    ) -> Result<(Operand, i64), XuloError> {
        let func_idx = self.current_func.unwrap();
        Ok(match lit {
            Literal::Int(n) => (Operand::Const(IrValue::I64(*n as i64)), 1),
            Literal::Float(n) => (Operand::Const(IrValue::F64(*n)), 2),
            Literal::String(s) => {
                let idx = self.module.add_string(s.clone());
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::Const {
                        dst: local,
                        value: IrValue::String(idx),
                    });
                (Operand::Local(local), 0)
            }
            Literal::Boolean(b) => (Operand::Const(IrValue::Bool(*b)), 5),
            Literal::Null => (Operand::Const(IrValue::Null), 6),
            Literal::List(_) => (Operand::Const(IrValue::Null), 3),
            Literal::Object(_) => (Operand::Const(IrValue::Null), 4),
        })
    }

    /// 枚举变体的 tag 分支：`EnumVariant(scrut) == idx` 为真走本臂，否则下一臂
    pub(super) fn emit_enum_tag_branch(
        &mut self,
        func_idx: usize,
        scrut: Operand,
        enum_name: &str,
        variant: &str,
        body_label: Label,
        next_label: Label,
    ) -> Result<(), XuloError> {
        let Some(variant_idx) = self.enum_variant_index(enum_name, variant) else {
            return Err(XuloError::new(
                ErrorKind::Semantic,
                format!("undefined variant: {}::{}", enum_name, variant),
            ));
        };
        let vt = self.module.functions[func_idx].add_local(IrType::I64);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(vt),
                func: RuntimeFn::EnumVariant,
                args: vec![scrut],
            });
        let cmp = self.module.functions[func_idx].add_local(IrType::Bool);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Eq {
                dst: cmp,
                left: Operand::Local(vt),
                right: Operand::Const(IrValue::I64(variant_idx as i64)),
            });
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Branch {
                condition: Operand::Local(cmp),
                then_label: body_label,
                else_label: next_label,
            });
        Ok(())
    }

    /// 一个 match 臂的求值：分支体的末表达式写入结果槽；f64 结果在按位
    /// 模式进 Pointer 槽前注册位模式
    pub(super) fn generate_match_arm_body(
        &mut self,
        arm: &MatchArm,
        result_local: LocalId,
    ) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();
        // 块内所有语句依序执行；末尾若是表达式语句，其值写入结果槽
        let (stmts, value_expr) = match &arm.value {
            MatchBody::Expr(e) => (Vec::new(), Some(e)),
            MatchBody::Block(b) => match b.statements.split_last() {
                Some((Statement::Expr(last), init)) => (init.to_vec(), Some(&last.expr)),
                _ => (b.statements.clone(), None),
            },
        };
        for stmt in &stmts {
            self.generate_statement(stmt)?;
        }
        if let Some(expr) = value_expr {
            let val = self.generate_expression(expr)?;
            if self.determine_element_tag(expr) == 2 {
                self.emit_register_float(val.clone());
            }
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::SetLocal {
                    dst: result_local,
                    src: val,
                });
        } else {
            // 无尾表达式（如 for 收尾）：结果槽置 null
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Const {
                    dst: result_local,
                    value: IrValue::I64(0),
                });
        }
        Ok(())
    }

    pub(super) fn tag_operand_for(&self, expr: &Expression, value: &Operand) -> Operand {
        if let Expression::Identifier { name, .. } = expr
            && let Some(tag) = self.dyn_tag_of.get(name)
        {
            return tag.clone();
        }
        if let Operand::Local(local) = value
            && let Some(tag) = self.value_tags.get(local)
        {
            return tag.clone();
        }
        Operand::Const(IrValue::I64(self.determine_element_tag(expr)))
    }

    fn stringify_key_operand(
        &mut self,
        expr: &Expression,
        value: Operand,
    ) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let tag = self.tag_operand_for(expr, &value);
        let local = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(local),
                func: RuntimeFn::Stringify,
                args: vec![value, tag],
            });
        Ok(Operand::Local(local))
    }

    fn generate_map_key(&mut self, key: &Expression) -> Result<Operand, XuloError> {
        let value = self.generate_expression(key)?;
        self.stringify_key_operand(key, value)
    }

    fn generate_map_member(
        &mut self,
        call: &Call,
        args: Vec<Operand>,
    ) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let method = call.method.as_deref().unwrap_or(&call.callee);
        let receiver = args
            .first()
            .cloned()
            .unwrap_or(Operand::Const(IrValue::Null));
        if method == "keys" || method == "values" || method == "entries" {
            let func = match method {
                "keys" => RuntimeFn::ObjectKeys,
                "values" => RuntimeFn::ObjectValues,
                _ => RuntimeFn::ObjectEntries,
            };
            let local = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func,
                    args: vec![receiver],
                });
            return Ok(Operand::Local(local));
        }
        let key = args
            .get(1)
            .cloned()
            .unwrap_or(Operand::Const(IrValue::Null));
        let key = self.stringify_key_operand(&call.arguments[0].value, key)?;
        match method {
            "set" => {
                let value = args
                    .get(2)
                    .cloned()
                    .unwrap_or(Operand::Const(IrValue::Null));
                let value_tag = self.tag_operand_for(&call.arguments[1].value, &value);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: None,
                        func: RuntimeFn::ObjectSet,
                        args: vec![receiver, key, value, value_tag],
                    });
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                Ok(Operand::Local(local))
            }
            "has" => {
                let local = self.module.functions[func_idx].add_local(IrType::Bool);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(local),
                        func: RuntimeFn::ObjectHas,
                        args: vec![receiver, key],
                    });
                Ok(Operand::Local(local))
            }
            "get" | "remove" => {
                let tag = self.module.functions[func_idx].add_local(IrType::I64);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(tag),
                        func: RuntimeFn::ObjectGetTag,
                        args: vec![receiver.clone(), key.clone()],
                    });
                let value = self.module.functions[func_idx].add_local(IrType::Pointer);
                let func = if method == "get" {
                    RuntimeFn::ObjectGet
                } else {
                    RuntimeFn::ObjectRemove
                };
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(value),
                        func,
                        args: vec![receiver, key],
                    });
                self.value_tags.insert(value, Operand::Local(tag));
                Ok(Operand::Local(value))
            }
            _ => Err(XuloError::new(
                ErrorKind::Semantic,
                format!("undefined function: {}", method),
            )),
        }
    }

    fn generate_set_member(
        &mut self,
        call: &Call,
        args: Vec<Operand>,
    ) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let method = call.method.as_deref().unwrap_or(&call.callee);
        let receiver = args
            .first()
            .cloned()
            .unwrap_or(Operand::Const(IrValue::Null));
        let value = args
            .get(1)
            .cloned()
            .unwrap_or(Operand::Const(IrValue::Null));
        let value_tag = self.tag_operand_for(&call.arguments[0].value, &value);
        let func = match method {
            "has" => RuntimeFn::SetHas,
            "insert" => RuntimeFn::SetInsert,
            "remove" => RuntimeFn::SetRemove,
            _ => {
                return Err(XuloError::new(
                    ErrorKind::Semantic,
                    format!("undefined function: {method}"),
                ));
            }
        };
        let local = self.module.functions[func_idx].add_local(IrType::Bool);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(local),
                func,
                args: vec![receiver, value, value_tag],
            });
        Ok(Operand::Local(local))
    }

    pub(super) fn generate_member_access(
        &mut self,
        member: &MemberAccess,
    ) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let object = self.generate_expression(&member.object)?;
        // Tuple positional access `p.0` reads the element by position (a
        // tuple is a list at run time).
        if matches!(
            self.infer_source_type_from_expr(&member.object),
            Some(Type::Tuple(_))
        ) && let Ok(index) = member.property.parse::<usize>()
        {
            let local = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::GetIndex {
                    dst: local,
                    array: object,
                    index: Operand::Const(IrValue::I64(index as i64)),
                });
            return Ok(Operand::Local(local));
        }
        // Read-only collection properties (spec `types/composite-types.md`,
        // `types/primitive-types.md`): `.len` on a `List`, `.size` on a
        // `Map`/`Set`, `.byte_len` on a `string`.
        let is_builder = matches!(
            self.infer_source_type_from_expr(&member.object),
            Some(Type::Named(name, _)) if name == "StringBuilder"
        );
        let prop_fn = match member.property.as_str() {
            "len" if is_builder => Some(RuntimeFn::StringBuilderLen),
            "byte_len" if is_builder => Some(RuntimeFn::StringBuilderByteLen),
            "len"
                if self.expr_is_list_receiver(&Some(Box::new(member.object.clone())))
                    || self.expr_is_fixed_array(&member.object) =>
            {
                Some(RuntimeFn::ArrayLen)
            }
            "size"
                if matches!(
                    self.infer_source_type_from_expr(&member.object),
                    Some(Type::Named(name, _)) if name == "Map"
                ) || matches!(
                    self.infer_source_type_from_expr(&member.object),
                    Some(Type::Object | Type::ObjectType(_))
                ) =>
            {
                Some(RuntimeFn::ObjectSize)
            }
            "size"
                if matches!(
                    self.infer_source_type_from_expr(&member.object),
                    Some(Type::Named(name, _)) if name == "Set"
                ) =>
            {
                Some(RuntimeFn::ArrayLen)
            }
            "len" => Some(RuntimeFn::StringSize),
            "byte_len" => Some(RuntimeFn::StringByteLen),
            _ => None,
        };
        if let Some(func) = prop_fn {
            let local = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(local),
                    func,
                    args: vec![object],
                });
            return Ok(Operand::Local(local));
        }
        let local = self.module.functions[func_idx].add_local(IrType::Pointer);

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::GetField {
                dst: local,
                object,
                field: member.property.clone(),
            });

        Ok(Operand::Local(local))
    }

    /// `e is T` / `e is not T`: read the value's runtime tag and compare it
    /// with the tag set the tested type names (spec
    /// `expressions/operators.md`, "Type tests").
    pub(super) fn generate_is(&mut self, test: &IsExpr) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let value = self.generate_expression(&test.expr)?;
        let result = self.module.functions[func_idx].add_local(IrType::Bool);
        match self.type_test_tags(&test.type_) {
            // `is unknown` is always true.
            None => {
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::Const {
                        dst: result,
                        value: IrValue::Bool(true),
                    });
            }
            Some(tags) if tags.is_empty() => {
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::Const {
                        dst: result,
                        value: IrValue::Bool(false),
                    });
            }
            Some(tags) => {
                let tag_local = self.module.functions[func_idx].add_local(IrType::I64);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(tag_local),
                        func: RuntimeFn::DetectTag,
                        args: vec![value],
                    });
                let mut accumulator: Option<LocalId> = None;
                for tag in tags {
                    let matched = self.module.functions[func_idx].add_local(IrType::Bool);
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Eq {
                            dst: matched,
                            left: Operand::Local(tag_local),
                            right: Operand::Const(IrValue::I64(tag)),
                        });
                    accumulator = Some(match accumulator {
                        None => matched,
                        Some(previous) => {
                            let combined = self.module.functions[func_idx].add_local(IrType::Bool);
                            self.module.functions[func_idx]
                                .instructions
                                .push(Instruction::Or {
                                    dst: combined,
                                    left: Operand::Local(previous),
                                    right: Operand::Local(matched),
                                });
                            combined
                        }
                    });
                }
                let combined = accumulator.expect("at least one tag");
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::SetLocal {
                        dst: result,
                        src: Operand::Local(combined),
                    });
            }
        }
        if !test.negated {
            return Ok(Operand::Local(result));
        }
        let negated = self.module.functions[func_idx].add_local(IrType::Bool);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Not {
                dst: negated,
                operand: Operand::Local(result),
            });
        Ok(Operand::Local(negated))
    }

    pub(super) fn generate_index(&mut self, index: &IndexExpr) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let indexed_string = matches!(
            self.infer_source_type_from_expr(&index.object),
            Some(Type::String)
        );
        let static_tag = self.determine_element_tag(&Expression::Index(Box::new(index.clone())));
        let array = self.generate_expression(&index.object)?;
        let idx = self.generate_expression(&index.index)?;
        let local = self.module.functions[func_idx].add_local(IrType::Pointer);

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::GetIndex {
                dst: local,
                array: array.clone(),
                index: idx.clone(),
            });
        // 元素标签静态未知时，在取值点从数组的配套标签槽读出；之后即使
        // 列表被改写，该结果值的标签也保持取值时的状态。
        if !indexed_string && static_tag == -1 {
            let tag = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(tag),
                    func: RuntimeFn::ArrayGetTag,
                    args: vec![array, idx],
                });
            self.value_tags.insert(local, Operand::Local(tag));
        }

        Ok(Operand::Local(local))
    }
}
