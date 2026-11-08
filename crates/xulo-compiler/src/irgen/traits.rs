use super::*;

impl IrGenerator {
    /// `std::fmt::format` 的渲染描述表：每个参数四项
    /// `[kind, display, debug, 0]`（kind 0=非用户值、1=struct、2=enum）。
    /// 只对字面量参数表有静态类型信息；其他形态补 NULL，由运行时按输出表
    /// 渲染。
    pub(super) fn fmt_meta_operand(&mut self, elements: Option<&[Expression]>) -> Operand {
        let Some(elements) = elements else {
            return Operand::Const(IrValue::Null);
        };
        let func_idx = self.current_func.unwrap();
        let mut rows: Vec<(i64, Option<FuncId>, Option<FuncId>)> = Vec::new();
        let mut any_user = false;
        for elem in elements {
            let Some(Type::Named(name, _)) = self.infer_source_type_from_expr(elem) else {
                rows.push((0, None, None));
                continue;
            };
            let resolved = self.resolve_type(&name);
            let decl_name = resolved
                .rsplit("::")
                .next()
                .unwrap_or(&resolved)
                .to_string();
            let builtin = matches!(
                decl_name.as_str(),
                "Format" | "Error" | "Result" | "Request" | "Response" | "Server"
            );
            if builtin {
                rows.push((0, None, None));
                continue;
            }
            let is_enum = self.enums.contains_key(&resolved)
                || self.enums.contains_key(&decl_name);
            let is_struct = self.structs.contains_key(&resolved)
                || self.structs.contains_key(&decl_name);
            if !is_enum && !is_struct {
                rows.push((0, None, None));
                continue;
            }
            any_user = true;
            let display = self
                .trait_impls
                .get(&impl_fn_name("Display", &decl_name, "display"))
                .copied();
            let debug = self
                .trait_impls
                .get(&impl_fn_name("Debug", &decl_name, "debug"))
                .copied();
            rows.push((if is_struct { 1 } else { 2 }, display, debug));
        }
        if !any_user {
            return Operand::Const(IrValue::Null);
        }
        let arr = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(arr),
                func: RuntimeFn::AllocArray,
                args: vec![Operand::Const(IrValue::I64((rows.len() * 4) as i64))],
            });
        for (kind, display, debug) in rows {
            let mut values = vec![Operand::Const(IrValue::I64(kind)), Operand::Const(IrValue::I64(0))];
            if let Some(fid) = display {
                let addr = self.module.functions[func_idx].add_local(IrType::I64);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::FuncAddr { dst: addr, func: fid });
                values[1] = Operand::Local(addr);
            }
            let debug_op = if let Some(fid) = debug {
                let addr = self.module.functions[func_idx].add_local(IrType::I64);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::FuncAddr { dst: addr, func: fid });
                Operand::Local(addr)
            } else {
                Operand::Const(IrValue::I64(0))
            };
            values.insert(2, debug_op);
            values.push(Operand::Const(IrValue::I64(0)));
            for value in values {
                self.module.functions[func_idx].instructions.push(
                    Instruction::RuntimeCall {
                        dst: None,
                        func: RuntimeFn::ArrayPush,
                        args: vec![Operand::Local(arr), value, Operand::Const(IrValue::I64(1))],
                    },
                );
            }
        }
        Operand::Local(arr)
    }

    /// 构造默认 `Format(precision: null, alternate: false)` 的对象操作数
    /// （spec `types/traits.md`，`display`/`debug` 的默认上下文）。
    pub(super) fn default_format_operand(&mut self) -> Operand {
        let func_idx = self.current_func.unwrap();
        let precision = Operand::Const(IrValue::Null);
        let alternate = Operand::Const(IrValue::I64(0));
        let dst = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::NewObject {
                dst,
                fields: vec![
                    ("precision".to_string(), precision, 6),
                    ("alternate".to_string(), alternate, 5),
                ],
            });
        Operand::Local(dst)
    }

    /// 若 `expr` 的静态类型实现了 `Display`（先）或 `Debug`（后），生成一次
    /// 默认 `Format` 的渲染调用并返回字符串操作数；否则 `None`，让 `print`
    /// 走输出表。与解释器 `render_print` 的用户类型咨询一致。
    pub(super) fn user_print_operand(
        &mut self,
        expr: &Expression,
        value: Operand,
    ) -> Option<Operand> {
        let Some(Type::Named(name, _)) = self.infer_source_type_from_expr(expr) else {
            return None;
        };
        let resolved = self.resolve_type(&name);
        let decl_name = resolved
            .rsplit("::")
            .next()
            .unwrap_or(&resolved)
            .to_string();
        let fid = self
            .trait_impls
            .get(&impl_fn_name("Display", &decl_name, "display"))
            .copied()
            .or_else(|| {
                self.trait_impls
                    .get(&impl_fn_name("Debug", &decl_name, "debug"))
                    .copied()
            })?;
        let format = self.default_format_operand();
        let func_idx = self.current_func.unwrap();
        let dst = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Call {
                dst: Some(dst),
                func: fid,
                args: vec![value, format],
            });
        Some(Operand::Local(dst))
    }

    /// 声明是否带 `#[derive(Debug)]`（与语义层/解释器同一判定）
    pub(super) fn derives_debug(attributes: &[Attribute]) -> bool {
        attributes
            .iter()
            .any(|attr| {
                attr.name == "derive"
                    && attr
                        .identifier_args()
                        .iter()
                        .any(|(arg, _)| *arg == "Debug")
            })
    }

    /// 取出一条可派生声明的（类型名、属性、目标）
    pub(super) fn derived_decl(stmt: &Statement) -> Option<(&str, &[Attribute], DerivedDecl<'_>)> {
        match stmt {
            Statement::Struct(def) => {
                Some((&def.name, &def.attributes, DerivedDecl::Struct(&def.fields)))
            }
            Statement::Enum(def) => {
                Some((&def.name, &def.attributes, DerivedDecl::Enum(&def.variants)))
            }
            Statement::Export(export) => match &export.item {
                ExportItem::Struct(def) => {
                    Some((&def.name, &def.attributes, DerivedDecl::Struct(&def.fields)))
                }
                ExportItem::Enum(def) => {
                    Some((&def.name, &def.attributes, DerivedDecl::Enum(&def.variants)))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// 注册 `#[derive(Debug)]` 的派生函数：IR 名为派发名
    /// `impl_Debug_{Type}_debug`，签名 `(self: ptr, format: ptr) -> string`。
    /// 结构体的派生同时挂到 `Type::debug`（成员调用 `u.debug(fmt)`
    /// 走与手写 impl 相同的既有解析路径），枚举只有派发名——与解释器
    /// 的注册位置一致。
    pub(super) fn register_derived_debug(&mut self, type_name: &str, is_struct: bool) -> FuncId {
        let mangled = impl_fn_name("Debug", type_name, "debug");
        // 返回字符串：Pointer 槽承载 C 字符串
        let func = IrFunction::new(
            mangled.clone(),
            vec![(LocalId(0), IrType::Pointer), (LocalId(1), IrType::Pointer)],
            IrType::Pointer,
        );
        let fid = self.module.add_function(func);
        self.func_return_types
            .insert(mangled.clone(), Some(Type::String));
        self.trait_impls.insert(mangled, fid);
        if is_struct {
            let type_key = self.resolve_type(type_name);
            let method_key = format!("{type_key}::debug");
            self.methods.insert(method_key.clone(), fid);
            // 成员调用 `u.debug(fmt)` 的返回标签也按这个键查表
            self.func_return_types
                .insert(method_key, Some(Type::String));
            self.method_types
                .entry("debug".to_string())
                .or_default()
                .push(type_key);
        }
        fid
    }

    /// 派生渲染一个字段/payload 值：Named 类型有 Debug 派发时递归调用
    /// 其实现（手写或派生，转发 format），否则按调用方给的运行时标签做
    /// 结构化渲染（枚举无 Debug 时落到输出表的枚举行）。
    pub(super) fn derive_render(
        &mut self,
        value: Operand,
        tag: Operand,
        ty: &Type,
        format: Operand,
    ) -> Operand {
        let func_idx = self.current_func.unwrap();
        if let Type::Named(name, _) = ty {
            // 派发名用声明名（语义层如此 mangling；别名导入取解析后的末段）
            let resolved = self.resolve_type(name);
            let decl_name = resolved
                .rsplit("::")
                .next()
                .unwrap_or(&resolved)
                .to_string();
            if let Some(&fid) = self
                .trait_impls
                .get(&impl_fn_name("Debug", &decl_name, "debug"))
            {
                let dst = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::Call {
                        dst: Some(dst),
                        func: fid,
                        args: vec![value, format],
                    });
                return Operand::Local(dst);
            }
        }
        let dst = self.module.functions[func_idx].add_local(IrType::Pointer);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(dst),
                func: RuntimeFn::Stringify,
                args: vec![value, tag],
            });
        Operand::Local(dst)
    }

    /// 派生 struct 的 `debug` 正文：`Type(name: v, …)`，字段按声明序，
    /// 每段 `名字: 渲染值` 用 `", "` 连接（spec `types/traits.md` 的派生
    /// 示例，与解释器 `render_derived_struct` 一致）。
    pub(super) fn generate_derived_struct_body(
        &mut self,
        type_name: &str,
        fields: &[StructField],
    ) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();
        let self_op = Operand::Local(LocalId(0));
        let format_op = Operand::Local(LocalId(1));
        let mut cur = self.const_string_local(&format!("{type_name}("));
        for (i, field) in fields.iter().enumerate() {
            if i > 0 {
                let sep = self.const_string_local(", ");
                cur = self.string_concat(cur, sep);
            }
            let label = self.const_string_local(&format!("{}: ", field.name));
            cur = self.string_concat(cur, label);
            let value = self.module.functions[func_idx].add_local(IrType::Pointer);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::GetField {
                    dst: value,
                    object: self_op.clone(),
                    field: field.name.clone(),
                });
            let name_op = self.const_string_local(&field.name);
            let tag = self.module.functions[func_idx].add_local(IrType::I64);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(tag),
                    func: RuntimeFn::ObjectGetTag,
                    args: vec![self_op.clone(), name_op],
                });
            let rendered = self.derive_render(
                Operand::Local(value),
                Operand::Local(tag),
                &field.type_,
                format_op.clone(),
            );
            cur = self.string_concat(cur, rendered);
        }
        let close = self.const_string_local(")");
        let result = self.string_concat(cur, close);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Return(Some(result)));
        Ok(())
    }

    /// 派生 enum 的 `debug` 正文：逐个变体测试 `EnumVariant(self) == i`，
    /// 渲染 `Type::Variant` 或 `Type::Variant(p0, …)`；payload 槽的标签取
    /// `EnumGetTag`（构造侧写入的真实标签）。
    pub(super) fn generate_derived_enum_body(
        &mut self,
        type_name: &str,
        variants: &[EnumVariant],
    ) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();
        let self_op = Operand::Local(LocalId(0));
        let format_op = Operand::Local(LocalId(1));
        let result_local = self.module.functions[func_idx].add_local(IrType::Pointer);
        let end_label = self.module.functions[func_idx].add_label();

        if variants.is_empty() {
            // 无变体的枚举：解释器退化为结构性 format
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(result_local),
                    func: RuntimeFn::Stringify,
                    args: vec![self_op, Operand::Const(IrValue::I64(7))],
                });
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Return(Some(Operand::Local(result_local))));
            return Ok(());
        }

        let vt = self.module.functions[func_idx].add_local(IrType::I64);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(vt),
                func: RuntimeFn::EnumVariant,
                args: vec![self_op.clone()],
            });

        // 前 n-1 个变体逐个测试；最后一个变体作为落空默认分支
        let mut tested: Vec<(Label, usize)> = Vec::new();
        for i in 0..variants.len() - 1 {
            let body_label = self.module.functions[func_idx].add_label();
            let next_label = self.module.functions[func_idx].add_label();
            let cmp = self.module.functions[func_idx].add_local(IrType::Bool);
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Eq {
                    dst: cmp,
                    left: Operand::Local(vt),
                    right: Operand::Const(IrValue::I64(i as i64)),
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
                .push(Instruction::Label(next_label));
            tested.push((body_label, i));
        }

        let last = variants.len() - 1;
        self.render_derived_variant(
            type_name,
            &variants[last],
            &self_op,
            &format_op,
            result_local,
        );
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Jump(end_label));

        for (body_label, i) in tested {
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Label(body_label));
            self.render_derived_variant(
                type_name,
                &variants[i],
                &self_op,
                &format_op,
                result_local,
            );
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Jump(end_label));
        }

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(end_label));
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Return(Some(Operand::Local(result_local))));
        Ok(())
    }

    /// 渲染一个枚举变体到结果槽：`Type::Variant` 或 `Type::Variant(p0, …)`
    pub(super) fn render_derived_variant(
        &mut self,
        type_name: &str,
        variant: &EnumVariant,
        self_op: &Operand,
        format_op: &Operand,
        result_local: LocalId,
    ) {
        let func_idx = self.current_func.unwrap();
        let mut cur = self.const_string_local(&format!("{type_name}::{}", variant.name));
        let payload: &[EnumPayloadParam] = variant.payload.as_deref().unwrap_or(&[]);
        if !payload.is_empty() {
            let open = self.const_string_local("(");
            cur = self.string_concat(cur, open);
            for (j, param) in payload.iter().enumerate() {
                if j > 0 {
                    let sep = self.const_string_local(", ");
                    cur = self.string_concat(cur, sep);
                }
                let value = self.module.functions[func_idx].add_local(IrType::Pointer);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(value),
                        func: RuntimeFn::EnumGet,
                        args: vec![self_op.clone(), Operand::Const(IrValue::I64(j as i64))],
                    });
                let tag = self.module.functions[func_idx].add_local(IrType::I64);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(tag),
                        func: RuntimeFn::EnumGetTag,
                        args: vec![self_op.clone(), Operand::Const(IrValue::I64(j as i64))],
                    });
                let rendered = self.derive_render(
                    Operand::Local(value),
                    Operand::Local(tag),
                    &param.type_,
                    format_op.clone(),
                );
                cur = self.string_concat(cur, rendered);
            }
            let close = self.const_string_local(")");
            cur = self.string_concat(cur, close);
        }
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::SetLocal {
                dst: result_local,
                src: cur,
            });
    }

    /// 阶段 3：登记 `#[derive(Debug)]` 的 trait 派发名（函数体留给
    /// [`Self::generate_derived_bodies`] 在所有注册完成后统一生成）。
    pub(super) fn register_derived_decls(&mut self, programs: &[(String, &Program)]) {
        for (id, program) in programs {
            self.current_module = id.clone();
            for stmt in &program.statements {
                let Some((type_name, attributes, decl)) = Self::derived_decl(stmt) else {
                    continue;
                };
                if Self::derives_debug(attributes) {
                    let is_struct = matches!(decl, DerivedDecl::Struct(_));
                    self.register_derived_debug(type_name, is_struct);
                }
            }
        }
    }

    /// 阶段 4：注册 `impl` 方法（固有方法 + trait 方法），并记录跨模块
    /// 可见的 `Trait::method` 派发表项。
    pub(super) fn register_impl_methods(
        &mut self,
        programs: &[(String, &Program)],
    ) -> Result<(), XuloError> {
        for (id, program) in programs {
            self.current_module = id.clone();
            for stmt in &program.statements {
                let Statement::Impl(imp) = stmt else { continue };
                let type_key = self.resolve_type(&imp.type_name);
                for m in &imp.methods {
                    let key = format!("{type_key}::{}", m.name);
                    let fid = self.register_named_function(&key, m)?;
                    self.methods.insert(key.clone(), fid);
                    if !imp.is_inherent {
                        self.trait_impls
                            .insert(impl_fn_name(&imp.trait_name, &imp.type_name, &m.name), fid);
                    }
                    self.method_types
                        .entry(m.name.clone())
                        .or_default()
                        .push(type_key.clone());
                    self.self_types
                        .insert(key, Type::Named(imp.type_name.clone(), Vec::new()));
                }
            }
        }
        Ok(())
    }

    /// 阶段 8：生成 `#[derive(Debug)]` 的函数正文（在所有目标注册完成后
    /// 进行，嵌套派生/手写 Debug 的互相引用都能解析）。
    pub(super) fn generate_derived_bodies(
        &mut self,
        programs: &[(String, &Program)],
    ) -> Result<(), XuloError> {
        for (id, program) in programs {
            self.current_module = id.clone();
            for stmt in &program.statements {
                let Some((type_name, attributes, decl)) = Self::derived_decl(stmt) else {
                    continue;
                };
                if !Self::derives_debug(attributes) {
                    continue;
                }
                let mangled = impl_fn_name("Debug", type_name, "debug");
                let Some(&fid) = self.trait_impls.get(&mangled) else {
                    continue;
                };
                self.current_func = Some(fid.0);
                self.locals.clear();
                self.local_types.clear();
                self.source_types.clear();
                self.label_map.clear();
                self.dyn_tag_of.clear();
                self.boxed_names.clear();
                self.boxed_logical.clear();
                self.closure_env = None;
                // 派生函数的接收者参数：`self` 是 LocalId(0)
                self.locals.insert("self".to_string(), LocalId(0));
                match decl {
                    DerivedDecl::Struct(fields) => {
                        self.generate_derived_struct_body(type_name, fields)?;
                    }
                    DerivedDecl::Enum(variants) => {
                        self.generate_derived_enum_body(type_name, variants)?;
                    }
                }
            }
        }
        Ok(())
    }
}
