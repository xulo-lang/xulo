use super::*;

impl IrGenerator {
    /// 分析一个字面量函数作用域：本层绑定、自由变量与需装箱的名字。
    /// 不进入嵌套字面量的函数体——它们的自由变量经递归分析回传。
    pub(super) fn analyze_scope(&self, params: &[Param], body: &Block) -> LiteralCapture {
        let mut binds: Vec<String> = Vec::new();
        for p in params {
            if !binds.contains(&p.name) {
                binds.push(p.name.clone());
            }
        }
        let mut refs: Vec<String> = Vec::new();
        let mut children_free: Vec<Vec<String>> = Vec::new();
        self.collect_scope_stmts(&body.statements, &mut binds, &mut refs, &mut children_free);

        // 自由变量 =（本层引用 ∪ 嵌套字面量的自由变量）− 本层绑定，
        // 按首次出现顺序去重
        let mut free: Vec<String> = Vec::new();
        for name in refs.iter().chain(children_free.iter().flatten()) {
            if !binds.contains(name) && !free.contains(name) {
                free.push(name.clone());
            }
        }
        // 需装箱 = 被嵌套字面量自由引用的本层绑定
        let mut boxed = HashSet::new();
        for name in children_free.iter().flatten() {
            if binds.contains(name) {
                boxed.insert(name.clone());
            }
        }
        LiteralCapture { free, boxed }
    }

    pub(super) fn collect_scope_stmts(
        &self,
        stmts: &[Statement],
        binds: &mut Vec<String>,
        refs: &mut Vec<String>,
        children_free: &mut Vec<Vec<String>>,
    ) {
        for stmt in stmts {
            self.collect_scope_stmt(stmt, binds, refs, children_free);
        }
    }

    pub(super) fn collect_scope_stmt(
        &self,
        stmt: &Statement,
        binds: &mut Vec<String>,
        refs: &mut Vec<String>,
        children_free: &mut Vec<Vec<String>>,
    ) {
        match stmt {
            Statement::Let(binding) => {
                push_unique(binds, &binding.name);
                if let Some(names) = &binding.tuple_names {
                    for n in names {
                        push_unique(binds, n);
                    }
                }
                if let Some(fields) = &binding.object_destructuring {
                    for (field, alias) in fields {
                        push_unique(binds, alias.as_deref().unwrap_or(field));
                    }
                }
                if let Some(v) = &binding.value {
                    self.collect_scope_expr(v, binds, refs, children_free);
                }
            }
            Statement::Fn(f) => {
                push_unique(binds, &f.name);
                let child = self.analyze_scope(&f.params, &f.body);
                children_free.push(child.free);
            }
            Statement::Return(r) => {
                if let Some(v) = &r.value {
                    self.collect_scope_expr(v, binds, refs, children_free);
                }
            }
            Statement::Throw(e) => {
                self.collect_scope_expr(e, binds, refs, children_free);
            }
            Statement::Expr(e) => {
                self.collect_scope_expr(&e.expr, binds, refs, children_free);
            }
            Statement::Block(b) => {
                self.collect_scope_stmts(&b.statements, binds, refs, children_free);
            }
            Statement::For(f) => {
                push_unique(binds, &f.iter_var);
                self.collect_scope_expr(&f.iterable, binds, refs, children_free);
                self.collect_scope_stmts(&f.body.statements, binds, refs, children_free);
            }
            Statement::While(w) => {
                self.collect_scope_expr(&w.condition, binds, refs, children_free);
                self.collect_scope_stmts(&w.body.statements, binds, refs, children_free);
            }
            Statement::Assign(a) => {
                match &a.target {
                    // 赋值目标是「写已存在的绑定」而非声明：被捕获时它是
                    // 自由引用（`fn(){ count = count + 1 }` 捕获外层 count）
                    AssignTarget::Name(n) => push_unique(refs, n),
                    AssignTarget::Member(obj, _) => {
                        self.collect_scope_expr(obj, binds, refs, children_free)
                    }
                    AssignTarget::Index(obj, idx) => {
                        self.collect_scope_expr(obj, binds, refs, children_free);
                        self.collect_scope_expr(idx, binds, refs, children_free);
                    }
                }
                self.collect_scope_expr(&a.value, binds, refs, children_free);
            }
            Statement::Try(t) => {
                push_unique(binds, &t.catch_var);
                self.collect_scope_stmts(&t.try_block.statements, binds, refs, children_free);
                self.collect_scope_stmts(&t.catch_block.statements, binds, refs, children_free);
            }
            Statement::Defer(d) => match &d.body {
                DeferBody::Block(b) => {
                    self.collect_scope_stmts(&b.statements, binds, refs, children_free)
                }
                DeferBody::Expr(e) => self.collect_scope_expr(e, binds, refs, children_free),
            },
            // 不参与 AOT lowering 的声明（struct/enum/use/组件等）无需分析
            _ => {}
        }
    }

    pub(super) fn collect_scope_expr(
        &self,
        expr: &Expression,
        binds: &mut Vec<String>,
        refs: &mut Vec<String>,
        children_free: &mut Vec<Vec<String>>,
    ) {
        match expr {
            Expression::Identifier { name, .. } => push_unique(refs, name),
            Expression::FnExpr(f) => {
                let child = self.analyze_scope(&f.params, &f.body);
                children_free.push(child.free);
            }
            Expression::Arrow(a) => {
                let block = match &a.body {
                    ArrowBody::Expr(e) => Block {
                        statements: vec![Statement::Expr(ExprStmt {
                            expr: e.clone(),
                            has_semicolon: false,
                            span: e.span().clone(),
                        })],
                    },
                    ArrowBody::Block(b) => b.clone(),
                };
                let child = self.analyze_scope(&a.params, &block);
                children_free.push(child.free);
            }
            Expression::Call(call) => {
                if call.object.is_none() && call.method.is_none() {
                    push_unique(refs, &call.callee);
                }
                if let Some(obj) = &call.object {
                    self.collect_scope_expr(obj, binds, refs, children_free);
                }
                for arg in &call.arguments {
                    self.collect_scope_expr(&arg.value, binds, refs, children_free);
                }
            }
            Expression::CallValue(cv) => {
                self.collect_scope_expr(&cv.callee, binds, refs, children_free);
                for arg in &cv.arguments {
                    self.collect_scope_expr(&arg.value, binds, refs, children_free);
                }
            }
            Expression::BinaryOp(b) => {
                self.collect_scope_expr(&b.left, binds, refs, children_free);
                self.collect_scope_expr(&b.right, binds, refs, children_free);
            }
            Expression::Unary(u) => {
                self.collect_scope_expr(&u.operand, binds, refs, children_free);
            }
            Expression::If(i) => {
                self.collect_scope_expr(&i.condition, binds, refs, children_free);
                self.collect_scope_stmts(&i.then_branch.statements, binds, refs, children_free);
                if let Some(e) = &i.else_branch {
                    self.collect_scope_stmts(&e.statements, binds, refs, children_free);
                }
            }
            Expression::Ternary(t) => {
                self.collect_scope_expr(&t.condition, binds, refs, children_free);
                self.collect_scope_expr(&t.then_value, binds, refs, children_free);
                self.collect_scope_expr(&t.else_value, binds, refs, children_free);
            }
            Expression::Match(m) => {
                self.collect_scope_expr(&m.value, binds, refs, children_free);
                for arm in &m.arms {
                    self.collect_scope_pattern(&arm.pattern, binds);
                    if let Some(g) = &arm.guard {
                        self.collect_scope_expr(g, binds, refs, children_free);
                    }
                    match &arm.value {
                        MatchBody::Expr(e) => {
                            self.collect_scope_expr(e, binds, refs, children_free)
                        }
                        MatchBody::Block(b) => {
                            self.collect_scope_stmts(&b.statements, binds, refs, children_free)
                        }
                    }
                }
            }
            Expression::Member(m) => {
                self.collect_scope_expr(&m.object, binds, refs, children_free);
            }
            Expression::Index(i) => {
                self.collect_scope_expr(&i.object, binds, refs, children_free);
                self.collect_scope_expr(&i.index, binds, refs, children_free);
            }
            Expression::Nullish(n) => {
                self.collect_scope_expr(&n.left, binds, refs, children_free);
                self.collect_scope_expr(&n.right, binds, refs, children_free);
            }
            Expression::Range(r) => {
                self.collect_scope_expr(&r.start, binds, refs, children_free);
                self.collect_scope_expr(&r.end, binds, refs, children_free);
            }
            Expression::Await { expr, .. } => {
                self.collect_scope_expr(expr, binds, refs, children_free);
            }
            Expression::Spread { expr, .. } => {
                self.collect_scope_expr(expr, binds, refs, children_free);
            }
            Expression::Tuple { items, .. } => {
                for it in items {
                    self.collect_scope_expr(it, binds, refs, children_free);
                }
            }
            Expression::Is(i) => {
                self.collect_scope_expr(&i.expr, binds, refs, children_free);
            }
            Expression::Panic { expr, .. } => {
                self.collect_scope_expr(expr, binds, refs, children_free);
            }
            Expression::Propagate { expr, .. } => {
                self.collect_scope_expr(expr, binds, refs, children_free);
            }
            Expression::Spawn(s) => {
                if let SpawnTarget::On(e) = &s.target {
                    self.collect_scope_expr(e, binds, refs, children_free);
                }
                match &s.body {
                    SpawnBody::Async(b) => {
                        self.collect_scope_stmts(&b.statements, binds, refs, children_free)
                    }
                    SpawnBody::Service { fields, .. } => {
                        for f in fields {
                            self.collect_scope_expr(&f.value, binds, refs, children_free);
                        }
                    }
                }
            }
            Expression::Lock(l) => {
                push_unique(refs, &l.name);
                self.collect_scope_stmts(&l.body.statements, binds, refs, children_free);
            }
            // 字面量 / 枚举引用 / Binding / TypedMap / Component：无自由局部变量
            //（列表/对象字面量的元素仍需遍历）
            Expression::Literal { value, .. } => match value {
                Literal::List(elems) => {
                    for e in elems {
                        self.collect_scope_expr(e, binds, refs, children_free);
                    }
                }
                Literal::Object(fields) => {
                    for f in fields {
                        if let ObjectField::Field { value, .. } = f {
                            self.collect_scope_expr(value, binds, refs, children_free);
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    pub(super) fn collect_scope_pattern(&self, pattern: &MatchPattern, binds: &mut Vec<String>) {
        match pattern {
            MatchPattern::EnumPayload { bindings, .. } => {
                for b in bindings {
                    if b != "_" {
                        push_unique(binds, b);
                    }
                }
            }
            MatchPattern::Binding { name, .. } => push_unique(binds, name),
            MatchPattern::Struct { bindings, .. } => {
                for b in bindings {
                    self.collect_scope_pattern(b, binds);
                }
            }
            MatchPattern::TypePattern {
                binder: PatternBinder::Binding(name),
                ..
            } => push_unique(binds, name),
            MatchPattern::Or { patterns, .. } => {
                for p in patterns {
                    self.collect_scope_pattern(p, binds);
                }
            }
            _ => {}
        }
    }

    pub(super) fn generate_function_body(&mut self, f: &FnDef) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();
        self.dyn_tag_of.clear();
        self.closure_env = None;

        // 捕获分析：被嵌套字面量自由引用的绑定需要装箱（cell 承载）
        let capture = self.analyze_scope(&f.params, &f.body);
        self.boxed_names = capture.boxed;
        self.boxed_logical.clear();

        // 使用 register_function 中已创建的参数 LocalId
        for (i, param) in f.params.iter().enumerate() {
            let local_id = self.module.functions[func_idx].params[i].0;
            self.locals.insert(param.name.clone(), local_id);
            // 记录参数的源级类型
            if let Some(ann) = &param.type_annotation {
                self.source_types.insert(param.name.clone(), ann.clone());
            }
            // 被捕获的参数装箱：cell 槽复用参数 LocalId（codegen 先解析实参
            // 再写 dst，参数原值可安全地作为 AllocCell 的输入）
            if self.boxed_names.contains(&param.name) {
                let logical = self.type_to_ir(param.type_annotation.as_ref());
                self.boxed_logical.insert(param.name.clone(), logical);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(local_id),
                        func: RuntimeFn::AllocCell,
                        args: vec![Operand::Local(local_id)],
                    });
            }
        }

        // 生成函数体语句（含内嵌 fn：在声明点生成闭包值）
        let last_stmt_is_return = matches!(f.body.statements.last(), Some(Statement::Return(_)));
        for stmt in &f.body.statements {
            self.generate_statement(stmt)?;
        }

        // 如果没有显式的 return，根据返回类型处理
        if !last_stmt_is_return {
            // Check if last instruction is already a terminator (e.g. from if/else returning in both branches)
            let last_is_terminator = matches!(
                self.module.functions[func_idx].instructions.last(),
                Some(Instruction::Return(_))
            );
            if !last_is_terminator {
                let ret_type = self.type_to_ir(f.return_type.as_ref());
                if !ret_type.is_void() {
                    if let Some(Statement::Expr(expr_stmt)) = f.body.statements.last() {
                        let operand = self.generate_expression(&expr_stmt.expr)?;
                        if self.determine_element_tag(&expr_stmt.expr) == 2 {
                            self.emit_register_float(operand.clone());
                        }
                        self.module.functions[func_idx]
                            .instructions
                            .push(Instruction::Return(Some(operand)));
                    } else {
                        self.module.functions[func_idx]
                            .instructions
                            .push(Instruction::Return(None));
                    }
                } else {
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Return(None));
                }
            }
        }

        Ok(())
    }

    /// 顶层函数引用作函数值：生成（或复用）thunk 闭包，
    /// 把直接调用协议适配成 CallIndirect 的 `(env, a0..a7)` 协议
    pub(super) fn generate_func_value(&mut self, name: &str) -> Result<Operand, XuloError> {
        let thunk_id = self.get_or_create_thunk(name)?;
        let host = self.current_func.unwrap();
        let env_local = self.module.functions[host].add_local(IrType::Pointer);
        self.module.functions[host]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(env_local),
                func: RuntimeFn::AllocEnv,
                args: vec![Operand::Const(IrValue::I64(0))],
            });
        let dst = self.module.functions[host].add_local(IrType::Pointer);
        self.module.functions[host]
            .instructions
            .push(Instruction::MakeClosure {
                dst,
                func: thunk_id,
                env: Operand::Local(env_local),
            });
        Ok(Operand::Local(dst))
    }

    /// 为目标函数建立 thunk（每函数至多一个）：9 参闭包入口，丢弃 env 与
    /// 多余实参，直接转发前 n 个实参
    pub(super) fn get_or_create_thunk(&mut self, target_name: &str) -> Result<FuncId, XuloError> {
        if let Some(&fid) = self.func_thunks.get(target_name) {
            return Ok(fid);
        }
        let Some(target_idx) = self
            .module
            .functions
            .iter()
            .position(|f| f.name == target_name)
        else {
            return Err(XuloError::new(
                ErrorKind::Semantic,
                format!("undefined function: {}", target_name),
            ));
        };
        let n_params = self.module.functions[target_idx].params.len();
        if n_params > 8 {
            return Err(XuloError::new(
                ErrorKind::Semantic,
                format!(
                    "function {} has {} parameters; taking it as a value supports at most 8",
                    target_name, n_params
                ),
            ));
        }
        let ret = self.module.functions[target_idx].return_type.clone();
        let mut fparams = vec![(LocalId(0), IrType::Pointer)];
        for i in 1..=8 {
            fparams.push((LocalId(i), IrType::I64));
        }
        let mut func = IrFunction::new(format!("thunk${}", target_name), fparams, ret.clone());
        func.closure_entry = true;
        let call_args: Vec<Operand> = (0..n_params)
            .map(|i| Operand::Local(LocalId(i + 1)))
            .collect();
        let dst = if ret.is_void() {
            None
        } else {
            let l = func.add_local(ret.clone());
            Some(l)
        };
        func.instructions.push(Instruction::Call {
            dst,
            func: FuncId(target_idx),
            args: call_args,
        });
        func.instructions
            .push(Instruction::Return(dst.map(Operand::Local)));
        let fid = self.module.add_function(func);
        self.func_thunks.insert(target_name.to_string(), fid);
        Ok(fid)
    }

    /// 内嵌 `fn` 语句：在声明点生成闭包值（捕获当时作用域的自由变量）
    pub(super) fn generate_inner_fn(&mut self, f: &FnDef) -> Result<(), XuloError> {
        let name = f.name.clone();
        let is_boxed = self.boxed_names.contains(&name);
        let capture = self.analyze_scope(&f.params, &f.body);

        // 注册体函数（IR 名带作用域计数，避免同名冲突）。
        // 与 thunk/箭头一致：闭包入口签名固定 (env, a0..a7)，
        // 体函数只声明前 n 个实参槽（1..=n）
        let ir_name = format!("inner${}${}", name, self.inner_fn_counter);
        self.inner_fn_counter += 1;
        let mut params: Vec<(LocalId, IrType)> = vec![(LocalId(0), IrType::Pointer)];
        for (i, p) in f.params.iter().enumerate() {
            let mut ty = self.type_to_ir(p.type_annotation.as_ref());
            if matches!(ty, IrType::Void) {
                ty = IrType::I64;
            }
            params.push((LocalId(i + 1), ty));
        }
        let return_type = f
            .return_type
            .as_ref()
            .map(|t| self.type_to_ir(Some(t)))
            .unwrap_or(IrType::Void);
        self.func_return_types
            .insert(ir_name.clone(), f.return_type.clone());
        self.func_return_types
            .insert(name.clone(), f.return_type.clone());
        self.func_params.insert(
            name.clone(),
            f.params
                .iter()
                .map(|p| (p.name.clone(), p.default.clone()))
                .collect(),
        );
        let mut func = IrFunction::new(ir_name, params, return_type);
        func.closure_entry = true;
        let func_id = self.module.add_function(func);

        // 名字的本地槽：被捕获时先建 cell——自递归与兄弟闭包捕获都要在
        // MakeClosure 之前拿到 cell 指针
        let host = self.current_func.unwrap();
        let slot = if is_boxed {
            let cell = self.module.functions[host].add_local(IrType::Pointer);
            self.module.functions[host]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(cell),
                    func: RuntimeFn::AllocCell,
                    args: vec![Operand::Const(IrValue::Null)],
                });
            self.boxed_logical.insert(name.clone(), IrType::Pointer);
            cell
        } else {
            self.module.functions[host].add_local(IrType::Pointer)
        };
        self.locals.insert(name.clone(), slot);

        let closure = self.emit_closure_creation(func_id, &capture.free)?;
        if is_boxed {
            self.module.functions[host]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: None,
                    func: RuntimeFn::CellSet,
                    args: vec![Operand::Local(slot), closure],
                });
        } else {
            self.module.functions[host]
                .instructions
                .push(Instruction::SetLocal {
                    dst: slot,
                    src: closure,
                });
        }

        self.generate_closure_body(
            func_id.0,
            &f.params,
            &f.body,
            &capture,
            f.return_type.as_ref(),
        )?;
        Ok(())
    }

    /// 闭包/箭头字面量求值：建 env + MakeClosure（仍在外层作用域），
    /// 然后生成体函数
    pub(super) fn generate_closure_value(
        &mut self,
        params: &[Param],
        return_type: Option<&Type>,
        body: &Block,
        arrow_expr: Option<&Expression>,
    ) -> Result<Operand, XuloError> {
        // 箭头表达式体统一包成末表达式语句（尾返回逻辑与块体共用）
        let synthesized;
        let body = if let Some(e) = arrow_expr {
            synthesized = Block {
                statements: vec![Statement::Expr(ExprStmt {
                    expr: e.clone(),
                    has_semicolon: false,
                    span: e.span().clone(),
                })],
            };
            &synthesized
        } else {
            body
        };

        let capture = self.analyze_scope(params, body);

        let ir_name = format!("closure${}", self.inner_fn_counter);
        self.inner_fn_counter += 1;
        // 无返回类型标注：动态返回（尾表达式作返回值，指针槽 + 运行时检测）
        let ret_ty = match return_type {
            None => IrType::Pointer,
            Some(t) => self.type_to_ir(Some(t)),
        };
        let mut fparams = vec![(LocalId(0), IrType::Pointer)];
        for i in 1..=8 {
            fparams.push((LocalId(i), IrType::I64));
        }
        let mut func = IrFunction::new(ir_name, fparams, ret_ty);
        func.closure_entry = true;
        let func_id = self.module.add_function(func);

        let closure = self.emit_closure_creation(func_id, &capture.free)?;
        self.generate_closure_body(func_id.0, params, body, &capture, return_type)?;
        Ok(closure)
    }

    /// 在当前（外层）作用域构建捕获环境并打包闭包值
    pub(super) fn emit_closure_creation(
        &mut self,
        func_id: FuncId,
        free: &[String],
    ) -> Result<Operand, XuloError> {
        let host = self.current_func.unwrap();
        let env_local = self.module.functions[host].add_local(IrType::Pointer);
        self.module.functions[host]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(env_local),
                func: RuntimeFn::AllocEnv,
                args: vec![Operand::Const(IrValue::I64(free.len() as i64))],
            });
        for (i, name) in free.iter().enumerate() {
            let cell = self.locals.get(name).copied().ok_or_else(|| {
                XuloError::new(ErrorKind::Semantic, format!("undefined variable: {}", name))
            })?;
            self.module.functions[host]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: None,
                    func: RuntimeFn::EnvSet,
                    args: vec![
                        Operand::Local(env_local),
                        Operand::Const(IrValue::I64(i as i64)),
                        Operand::Local(cell),
                    ],
                });
        }
        let dst = self.module.functions[host].add_local(IrType::Pointer);
        self.module.functions[host]
            .instructions
            .push(Instruction::MakeClosure {
                dst,
                func: func_id,
                env: Operand::Local(env_local),
            });
        Ok(Operand::Local(dst))
    }

    /// 在闭包入口函数里生成函数体：加载 env 捕获、装箱参数、体语句与尾返回
    pub(super) fn generate_closure_body(
        &mut self,
        func_id: usize,
        params: &[Param],
        body: &Block,
        capture: &LiteralCapture,
        return_type: Option<&Type>,
    ) -> Result<(), XuloError> {
        // swap 前采集捕获名在外层的逻辑/源级类型
        let mut env_logical: HashMap<String, IrType> = HashMap::new();
        let mut env_source: Vec<(String, Type)> = Vec::new();
        for name in &capture.free {
            if let Some(t) = self.boxed_logical.get(name) {
                env_logical.insert(name.clone(), t.clone());
            }
            if let Some(t) = self.source_types.get(name) {
                env_source.push((name.clone(), t.clone()));
            }
        }

        let snap = self.swap_scope();
        self.current_func = Some(func_id);
        self.closure_env = Some(LocalId(0));
        self.boxed_names = capture.boxed.clone();
        for name in &capture.free {
            self.boxed_names.insert(name.clone());
        }
        self.boxed_logical = env_logical;

        // env 槽加载：cell 指针 → 本地 Pointer 槽
        for (i, name) in capture.free.iter().enumerate() {
            let cell = self.module.functions[func_id].add_local(IrType::Pointer);
            self.module.functions[func_id]
                .instructions
                .push(Instruction::RuntimeCall {
                    dst: Some(cell),
                    func: RuntimeFn::EnvGet,
                    args: vec![
                        Operand::Local(LocalId(0)),
                        Operand::Const(IrValue::I64(i as i64)),
                    ],
                });
            self.locals.insert(name.clone(), cell);
        }
        for (name, ty) in env_source {
            self.source_types.insert(name, ty);
        }

        // 参数绑定（LocalId(i+1)）；被捕获的参数装箱
        for (i, p) in params.iter().enumerate() {
            let local = LocalId(i + 1);
            self.locals.insert(p.name.clone(), local);
            if let Some(ann) = &p.type_annotation {
                self.source_types.insert(p.name.clone(), ann.clone());
            }
            if self.boxed_names.contains(&p.name) {
                let logical = self.type_to_ir(p.type_annotation.as_ref());
                self.boxed_logical.insert(p.name.clone(), logical);
                self.module.functions[func_id]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(local),
                        func: RuntimeFn::AllocCell,
                        args: vec![Operand::Local(local)],
                    });
            }
        }

        // 体语句（尾表达式若作为返回值则延后、只生成一次）；
        // 无返回类型标注 = 动态返回，尾表达式同样作为返回值
        let ret_type = self.type_to_ir(return_type);
        let returns_value = return_type.is_none() || !ret_type.is_void();
        let last_stmt_is_return = matches!(body.statements.last(), Some(Statement::Return(_)));
        let skip_last = returns_value
            && !last_stmt_is_return
            && matches!(body.statements.last(), Some(Statement::Expr(_)));
        let total = body.statements.len();
        for (i, stmt) in body.statements.iter().enumerate() {
            if skip_last && i + 1 == total {
                continue;
            }
            self.generate_statement(stmt)?;
        }

        if !last_stmt_is_return {
            let last_is_terminator = matches!(
                self.module.functions[func_id].instructions.last(),
                Some(Instruction::Return(_))
            );
            if !last_is_terminator {
                if returns_value {
                    if let Some(Statement::Expr(expr_stmt)) = body.statements.last() {
                        let operand = self.generate_expression(&expr_stmt.expr)?;
                        if self.determine_element_tag(&expr_stmt.expr) == 2 {
                            self.emit_register_float(operand.clone());
                        }
                        self.module.functions[func_id]
                            .instructions
                            .push(Instruction::Return(Some(operand)));
                    } else {
                        self.module.functions[func_id]
                            .instructions
                            .push(Instruction::Return(None));
                    }
                } else {
                    self.module.functions[func_id]
                        .instructions
                        .push(Instruction::Return(None));
                }
            }
        }

        self.restore_scope(snap);
        Ok(())
    }

    /// 保存外层作用域状态（进入闭包体前调用）
    pub(super) fn swap_scope(&mut self) -> ScopeSnapshot {
        ScopeSnapshot {
            current_func: self.current_func.take(),
            locals: std::mem::take(&mut self.locals),
            local_types: std::mem::take(&mut self.local_types),
            source_types: std::mem::take(&mut self.source_types),
            label_map: std::mem::take(&mut self.label_map),
            dyn_tag_of: std::mem::take(&mut self.dyn_tag_of),
            loop_stack: std::mem::take(&mut self.loop_stack),
            boxed_names: std::mem::take(&mut self.boxed_names),
            boxed_logical: std::mem::take(&mut self.boxed_logical),
            closure_env: self.closure_env.take(),
        }
    }

    /// 恢复外层作用域状态（闭包体生成完毕后调用）
    pub(super) fn restore_scope(&mut self, snap: ScopeSnapshot) {
        self.current_func = snap.current_func;
        self.locals = snap.locals;
        self.local_types = snap.local_types;
        self.source_types = snap.source_types;
        self.label_map = snap.label_map;
        self.dyn_tag_of = snap.dyn_tag_of;
        self.loop_stack = snap.loop_stack;
        self.boxed_names = snap.boxed_names;
        self.boxed_logical = snap.boxed_logical;
        self.closure_env = snap.closure_env;
    }
}
