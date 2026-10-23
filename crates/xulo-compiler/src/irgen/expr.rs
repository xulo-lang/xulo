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
            // 区间表达式 `0..<3` / `1...4` 求值为列表（for 循环已在
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

    pub(super) fn generate_literal(&mut self, lit: &Literal) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let (value, ty) = match lit {
            Literal::Number(n) => {
                if n.fract() == 0.0 {
                    (IrValue::I64(*n as i64), IrType::I64)
                } else {
                    (IrValue::F64(*n), IrType::F64)
                }
            }
            Literal::Boolean(b) => (IrValue::Bool(*b), IrType::Bool),
            Literal::String(s) => {
                let idx = self.module.add_string(s.clone());
                (IrValue::String(idx), IrType::Pointer)
            }
            Literal::Null => (IrValue::Null, IrType::Pointer),
            Literal::List(elements) => {
                // 创建数组
                let mut element_ops = Vec::new();
                for elem in elements {
                    let tag = self.determine_element_tag(elem);
                    element_ops.push((self.generate_expression(elem)?, tag));
                }
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::NewArray {
                        dst: local,
                        elements: element_ops,
                    });
                return Ok(Operand::Local(local));
            }
            Literal::Object(fields) => {
                // 创建对象
                let mut field_ops = Vec::new();
                for field in fields {
                    match field {
                        ObjectField::Field { name, value } => {
                            let tag = self.determine_element_tag(value);
                            field_ops.push((name.clone(), self.generate_expression(value)?, tag));
                        }
                        ObjectField::Spread { .. } => {
                            // Spread 暂不支持
                        }
                    }
                }
                let local = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::NewObject {
                        dst: local,
                        fields: field_ops,
                    });
                return Ok(Operand::Local(local));
            }
        };

        let local = self.module.functions[func_idx].add_local(ty);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Const { dst: local, value });
        Ok(Operand::Local(local))
    }

    pub(super) fn generate_binary_op(&mut self, binop: &BinaryOp) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let left = self.generate_expression(&binop.left)?;
        let right = self.generate_expression(&binop.right)?;

        // match 绑定参与的数值运算：payload 的 int/f64 标签运行时才知道，
        // 两侧先经 xulo_as_f64 归一到 f64
        let dyn_math = matches!(
            binop.operator,
            BinaryOperator::Add
                | BinaryOperator::Sub
                | BinaryOperator::Mul
                | BinaryOperator::Div
                | BinaryOperator::Lt
                | BinaryOperator::Gt
                | BinaryOperator::Lte
                | BinaryOperator::Gte
        ) && (self.expr_is_dyn(&binop.left) || self.expr_is_dyn(&binop.right));
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
        if matches!(binop.operator, BinaryOperator::Add)
            && (self.is_string_like(&binop.left) || self.is_string_like(&binop.right))
        {
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

        if matches!(binop.operator, BinaryOperator::Add)
            && (self.is_array_like_expr(&binop.left) || self.is_array_like_expr(&binop.right))
        {
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
            BinaryOperator::Mod => IrType::I64,
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
            UnaryOperator::BitNot => IrType::I64,
            UnaryOperator::Move | UnaryOperator::Copy => IrType::Pointer,
        };

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

        // `std::math::` / `std::time::` / `std::fs::` / `std::io::` /
        // `std::os::` / `std::path::` / `std::net::` 命名空间内建（语义层已按
        // builtin_member 校验成员存在；裸名调用已被 apply_std_calls 改写为
        // 限定形式）
        if call.object.is_none()
            && (call.callee.starts_with("std::math::")
                || call.callee.starts_with("std::time::")
                || call.callee.starts_with("std::fs::")
                || call.callee.starts_with("std::io::")
                || call.callee.starts_with("std::os::")
                || call.callee.starts_with("std::path::")
                || call.callee.starts_with("std::net::"))
        {
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
        // receiver 是用户结构体时不抢方法名（用户 impl 方法优先）
        let receiver_is_struct = call
            .object
            .as_ref()
            .is_some_and(|o| self.struct_name_of_expr(o).is_some());
        let runtime_fn = if call.object.is_some() {
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
                "to_uppercase" => Some(RuntimeFn::StringToUppercase),
                "to_lowercase" => Some(RuntimeFn::StringToLowercase),
                "trim" => Some(RuntimeFn::StringTrim),
                "starts_with" => Some(RuntimeFn::StringStartsWith),
                "ends_with" => Some(RuntimeFn::StringEndsWith),
                "split" => Some(RuntimeFn::StringSplit),
                "pad_start" => Some(RuntimeFn::StringPadStart),
                "pad_end" => Some(RuntimeFn::StringPadEnd),
                "repeat" => Some(RuntimeFn::StringRepeat),
                // `contains`/`length` 在列表与字符串上重名：列表接收者保持
                // 原路径（这些成员尚未 lowering，按用户函数解析报错）
                "contains" if !self.expr_is_list_receiver(&call.object) => {
                    Some(RuntimeFn::StringContains)
                }
                "length" if !self.expr_is_list_receiver(&call.object) => {
                    Some(RuntimeFn::StringLength)
                }
                _ => None,
            }
        } else {
            // 普通函数调用
            match call.callee.as_str() {
                "print" | "println" => {
                    // 使用 PrintValue 带类型标签
                    if args.is_empty() {
                        // print() 无参数: 传 (0, 6) 打印空行
                        args.push(Operand::Const(IrValue::I64(0)));
                        args.push(Operand::Const(IrValue::I64(6)));
                    } else {
                        // 从原始 AST 表达式推断类型标签（而非生成后的 operand）；
                        // match 绑定用伴随的运行时标签槽
                        let arg_expr = &call.arguments[0].value;
                        if let Expression::Identifier { name, .. } = arg_expr
                            && let Some(tag_op) = self.dyn_tag_of.get(name).cloned()
                        {
                            args.push(tag_op);
                        } else {
                            let tag = self.determine_element_tag(arg_expr);
                            args.push(Operand::Const(IrValue::I64(tag)));
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
                        if let Expression::Identifier { name, .. } = &arg.value
                            && let Some(tag_op) = self.dyn_tag_of.get(name).cloned()
                        {
                            tag_op
                        } else {
                            Operand::Const(IrValue::I64(self.determine_element_tag(&arg.value)))
                        }
                    } else {
                        Operand::Const(IrValue::I64(6)) // null
                    };
                    args.push(tag_op);
                    Some(RuntimeFn::Stringify)
                }
                _ => None,
            }
        };

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
            let tag = self.determine_element_tag(&arg.value);
            let s_local = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(s_local),
                    func: RuntimeFn::Stringify,
                    args: vec![v, Operand::Const(IrValue::I64(tag))],
                });
            let next = Operand::Local(s_local);
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
        // 简化处理：只取最后一个表达式
        if let Some(Statement::Expr(e)) = if_expr.then_branch.statements.last() {
            let val = self.generate_expression(&e.expr)?;
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::SetLocal {
                    dst: result_local,
                    src: val,
                });
        }
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Jump(end_label));

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(else_label));
        if let Some(else_branch) = &if_expr.else_branch
            && let Some(Statement::Expr(e)) = else_branch.statements.last()
        {
            let val = self.generate_expression(&e.expr)?;
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::SetLocal {
                    dst: result_local,
                    src: val,
                });
        }
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Jump(end_label));
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(end_label));

        Ok(Operand::Local(result_local))
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
            Literal::Number(n) => {
                // i64 槽只承载精确可表示的整数，其余按 f64 位模式比对
                if n.fract() == 0.0 && n.is_finite() && n.abs() <= 9.007_199_254_740_992e15 {
                    (Operand::Const(IrValue::I64(*n as i64)), 1)
                } else {
                    (Operand::Const(IrValue::F64(*n)), 2)
                }
            }
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

    pub(super) fn generate_member_access(
        &mut self,
        member: &MemberAccess,
    ) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let object = self.generate_expression(&member.object)?;
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

    pub(super) fn generate_index(&mut self, index: &IndexExpr) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();
        let array = self.generate_expression(&index.object)?;
        let idx = self.generate_expression(&index.index)?;
        let local = self.module.functions[func_idx].add_local(IrType::Pointer);

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::GetIndex {
                dst: local,
                array,
                index: idx,
            });

        Ok(Operand::Local(local))
    }
}
