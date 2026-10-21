use std::collections::{HashMap, HashSet};

use xulo_core::ast::*;
use xulo_core::error::{ErrorKind, XuloError};
use xulo_core::ir::*;

/// IR 生成器
pub struct IrGenerator {
    module: IrModule,
    current_func: Option<usize>,
    locals: HashMap<String, LocalId>,
    local_types: HashMap<LocalId, IrType>,
    /// 跟踪变量的源级类型（用于 print 等需要类型区分的场景）
    source_types: HashMap<String, Type>,
    /// 函数名 -> AST 返回类型注解（用于泛型函数的返回类型推断）
    func_return_types: HashMap<String, Option<Type>>,
    label_map: HashMap<String, Label>,
    loop_stack: Vec<LoopContext>,
    /// struct 定义（结构体构造需要字段顺序与默认值）
    structs: HashMap<String, Vec<StructField>>,
    /// `Type::method` -> IR 函数（impl 固有方法）
    methods: HashMap<String, FuncId>,
    /// 方法名 -> 定义它的类型列表（唯一时可按方法名分派）
    method_types: HashMap<String, Vec<String>>,
    /// IR 函数名 -> self 的源级类型（方法体里 self 字段的类型推断）
    self_types: HashMap<String, Type>,
    /// 返回值以 F64 位模式穿过整数签名的函数（Math::sqrt 落点，print 需要 tag=2）
    float_bit_returns: HashSet<String>,
    /// 枚举定义：类型名 -> 按声明序的变体名（序号即 match/构造的 tag）。
    /// 内置 `Result` 预置，模块自己的 `enum Result` 覆盖之。
    enums: HashMap<String, Vec<String>>,
    /// match 绑定名 -> 运行时标签操作数（payload 是 int 还是 f64 位模式，
    /// 编译期不知道，构造侧的 tag 在绑定时摘出来伴随使用）
    dyn_tag_of: HashMap<String, Operand>,
    /// 本作用域需要装箱（cell 承载）的名字：被嵌套字面量自由引用的绑定，
    /// 以及闭包体里来自 env 的捕获名
    boxed_names: HashSet<String>,
    /// 装箱名字的逻辑类型（I64/F64/Bool/Pointer）：cell 的本地槽是
    /// Pointer，读出时按逻辑类型还原算术/标签语义
    boxed_logical: HashMap<String, IrType>,
    /// 闭包体当前的 env 参数（LocalId(0)）；顶层函数体为 None
    closure_env: Option<LocalId>,
    /// 顶层函数名 -> 惰性生成的 thunk 闭包（把直接调用协议适配成
    /// CallIndirect 的 `(env, a0..a7)` 协议）
    func_thunks: HashMap<String, FuncId>,
    /// 内嵌 fn / 闭包体的 IR 名计数（同名函数在不同作用域需要唯一名）
    inner_fn_counter: usize,
}

struct LoopContext {
    break_label: Label,
    continue_label: Label,
}

/// 进入闭包体前保存的外层作用域状态
struct ScopeSnapshot {
    current_func: Option<usize>,
    locals: HashMap<String, LocalId>,
    local_types: HashMap<LocalId, IrType>,
    source_types: HashMap<String, Type>,
    label_map: HashMap<String, Label>,
    dyn_tag_of: HashMap<String, Operand>,
    loop_stack: Vec<LoopContext>,
    boxed_names: HashSet<String>,
    boxed_logical: HashMap<String, IrType>,
    closure_env: Option<LocalId>,
}

/// 一个字面量函数（fn 表达式 / 箭头 / 内嵌 fn 语句）的捕获分析结果
struct LiteralCapture {
    /// 自由变量（按首次出现顺序去重）——创建时推入 env，体入口处加载
    free: Vec<String>,
    /// 本作用域绑定且被嵌套字面量自由引用 → 需要装箱成 cell
    boxed: HashSet<String>,
}

impl Default for IrGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl IrGenerator {
    pub fn new() -> Self {
        IrGenerator {
            module: IrModule::new(),
            current_func: None,
            locals: HashMap::new(),
            local_types: HashMap::new(),
            source_types: HashMap::new(),
            func_return_types: HashMap::new(),
            label_map: HashMap::new(),
            loop_stack: Vec::new(),
            structs: HashMap::new(),
            methods: HashMap::new(),
            method_types: HashMap::new(),
            self_types: HashMap::new(),
            float_bit_returns: HashSet::new(),
            enums: HashMap::new(),
            dyn_tag_of: HashMap::new(),
            boxed_names: HashSet::new(),
            boxed_logical: HashMap::new(),
            closure_env: None,
            func_thunks: HashMap::new(),
            inner_fn_counter: 0,
        }
    }

    pub fn generate(&mut self, program: &Program) -> Result<IrModule, XuloError> {
        // 首先注册所有函数（但不生成函数体）
        for stmt in &program.statements {
            match stmt {
                Statement::Fn(f) => {
                    let _func_id = self.register_function(f)?;
                }
                Statement::Export(export) => {
                    if let ExportItem::Fn(f) = &export.item {
                        self.register_function(f)?;
                    }
                }
                _ => {}
            }
        }

        // 记录 struct 定义（结构体构造需要字段顺序与默认值）
        for stmt in &program.statements {
            if let Statement::Struct(def) = stmt {
                self.structs.insert(def.name.clone(), def.fields.clone());
            }
        }

        // 枚举定义：内置 Result 预置，模块自己的 enum 覆盖同名（spec：
        // Result 既是 prelude 类型，也是程序可以重定义的名字）
        self.enums.insert(
            "Result".to_string(),
            vec!["Ok".to_string(), "Err".to_string()],
        );
        for stmt in &program.statements {
            match stmt {
                Statement::Enum(def) => {
                    self.enums.insert(
                        def.name.clone(),
                        def.variants.iter().map(|v| v.name.clone()).collect(),
                    );
                }
                Statement::Export(export) => {
                    if let ExportItem::Enum(def) = &export.item {
                        self.enums.insert(
                            def.name.clone(),
                            def.variants.iter().map(|v| v.name.clone()).collect(),
                        );
                    }
                }
                _ => {}
            }
        }

        // 注册 impl 固有方法：按 `Type::method` 命名，避免与顶层函数重名冲突
        for stmt in &program.statements {
            let Statement::Impl(imp) = stmt else { continue };
            for m in &imp.methods {
                let key = format!("{}::{}", imp.type_name, m.name);
                let fid = self.register_named_function(&key, m)?;
                self.methods.insert(key.clone(), fid);
                self.method_types
                    .entry(m.name.clone())
                    .or_default()
                    .push(imp.type_name.clone());
                self.self_types
                    .insert(key, Type::Named(imp.type_name.clone(), Vec::new()));
            }
        }

        // 返回 Math::sqrt 位模式的函数：print 端要用 tag=2 还原小数
        for stmt in &program.statements {
            match stmt {
                Statement::Fn(f) => self.note_float_bit_returns(&f.name.clone(), f),
                Statement::Export(export) => {
                    if let ExportItem::Fn(f) = &export.item {
                        self.note_float_bit_returns(&f.name.clone(), f);
                    }
                }
                Statement::Impl(imp) => {
                    for m in &imp.methods {
                        let key = format!("{}::{}", imp.type_name, m.name);
                        self.note_float_bit_returns(&key, m);
                    }
                }
                _ => {}
            }
        }

        // 查找 main 函数索引
        let main_idx = self.module.functions.iter().position(|f| f.name == "main");

        if main_idx.is_none() {
            // 没有 main 函数，创建一个空的
            let main_func = IrFunction::new("main".to_string(), Vec::new(), IrType::Void);
            self.module.add_function(main_func);
        }

        self.module.entry_point = FuncId(main_idx.unwrap_or(self.module.functions.len() - 1));

        // Generate IR for ALL functions（含 impl 方法）
        let mut func_defs: Vec<(usize, FnDef)> = Vec::new();
        for stmt in &program.statements {
            match stmt {
                Statement::Fn(f) => {
                    if let Some(idx) = self
                        .module
                        .functions
                        .iter()
                        .position(|ir_f| ir_f.name == f.name)
                    {
                        func_defs.push((idx, f.clone()));
                    }
                }
                Statement::Export(export) => {
                    if let ExportItem::Fn(f) = &export.item
                        && let Some(idx) = self
                            .module
                            .functions
                            .iter()
                            .position(|ir_f| ir_f.name == f.name)
                    {
                        func_defs.push((idx, f.clone()));
                    }
                }
                Statement::Impl(imp) => {
                    for m in &imp.methods {
                        let key = format!("{}::{}", imp.type_name, m.name);
                        if let Some(idx) = self
                            .module
                            .functions
                            .iter()
                            .position(|ir_f| ir_f.name == key)
                        {
                            func_defs.push((idx, m.clone()));
                        }
                    }
                }
                _ => {}
            }
        }

        for (idx, func_def) in func_defs {
            self.current_func = Some(idx);
            self.locals.clear();
            self.local_types.clear();
            self.source_types.clear();
            self.label_map.clear();
            // impl 方法：self 的源级类型
            let fn_name = self.module.functions[idx].name.clone();
            if let Some(ty) = self.self_types.get(&fn_name) {
                self.source_types.insert("self".to_string(), ty.clone());
            }
            self.generate_function_body(&func_def)?;
        }

        Ok(self.module.clone())
    }

    /// 函数返回值是 Math::sqrt 的 F64 位模式时记下（print 需要 tag=2 还原小数）
    fn note_float_bit_returns(&mut self, ir_name: &str, f: &FnDef) {
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

    fn expr_is_sqrt_call(&self, expr: &Expression) -> bool {
        matches!(expr, Expression::Call(call)
            if call.object.is_none() && call.callee == "std::math::sqrt")
    }

    fn register_function(&mut self, f: &FnDef) -> Result<FuncId, XuloError> {
        self.register_named_function(&f.name.clone(), f)
    }

    /// 以指定的 IR 名注册函数（impl 方法用 `Type::method` 作名）
    fn register_named_function(&mut self, name: &str, f: &FnDef) -> Result<FuncId, XuloError> {
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

        let func = IrFunction::new(name.to_string(), params, return_type);
        Ok(self.module.add_function(func))
    }

    /// 分析一个字面量函数作用域：本层绑定、自由变量与需装箱的名字。
    /// 不进入嵌套字面量的函数体——它们的自由变量经递归分析回传。
    fn analyze_scope(&self, params: &[Param], body: &Block) -> LiteralCapture {
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

    fn collect_scope_stmts(
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

    fn collect_scope_stmt(
        &self,
        stmt: &Statement,
        binds: &mut Vec<String>,
        refs: &mut Vec<String>,
        children_free: &mut Vec<Vec<String>>,
    ) {
        match stmt {
            Statement::Let(binding) => {
                push_unique(binds, &binding.name);
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
                    AssignTarget::Name(n) => push_unique(binds, n),
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

    fn collect_scope_expr(
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

    fn collect_scope_pattern(&self, pattern: &MatchPattern, binds: &mut Vec<String>) {
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

    fn generate_function_body(&mut self, f: &FnDef) -> Result<(), XuloError> {
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

    fn generate_statement(&mut self, stmt: &Statement) -> Result<(), XuloError> {
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
                    let func_idx = self.current_func.unwrap();
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Jump(ctx.break_label));
                }
                Ok(())
            }
            Statement::Continue => {
                if let Some(ctx) = self.loop_stack.last() {
                    let func_idx = self.current_func.unwrap();
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Jump(ctx.continue_label));
                }
                Ok(())
            }
            _ => Ok(()), // 其他语句暂时忽略
        }
    }

    fn generate_let(&mut self, binding: &LetBinding) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();

        // 在生成表达式之前，先记录源级类型
        if let Some(ann) = &binding.type_annotation {
            self.source_types.insert(binding.name.clone(), ann.clone());
        } else if let Some(ref expr) = binding.value
            && let Some(src_ty) = self.infer_source_type_from_expr(expr)
        {
            self.source_types.insert(binding.name.clone(), src_ty);
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

            let init_val = self
                .generate_expression(binding.value.as_ref().unwrap())?;
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

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::SetLocal {
                dst: local,
                src: init_val,
            });

        Ok(())
    }

    fn infer_type_from_operand(&self, operand: &Operand, func_idx: usize) -> IrType {
        match operand {
            Operand::Const(val) => match val {
                IrValue::I64(_) => IrType::I64,
                IrValue::F64(_) => IrType::F64,
                IrValue::Bool(_) => IrType::Bool,
                IrValue::String(_) => IrType::Pointer,
                IrValue::Null => IrType::Pointer,
            },
            Operand::Local(local_id) => {
                // 查找局部变量的类型
                if let Some((_, ty)) = self.module.functions[func_idx]
                    .locals
                    .iter()
                    .find(|(id, _)| id == local_id)
                {
                    ty.clone()
                } else {
                    IrType::Pointer
                }
            }
        }
    }

    /// 检查操作数是否是数组类型（通过源级类型推断和指令分析）
    /// 检查表达式是否产生数组（列表字面量、列表变量、数组拼接等）
    fn is_array_like_expr(&self, expr: &Expression) -> bool {
        match expr {
            Expression::Literal { value, .. } => matches!(value, Literal::List(_)),
            Expression::Identifier { name, .. } => {
                if let Some(src_ty) = self.source_types.get(name) {
                    matches!(src_ty, Type::List(_))
                } else {
                    false
                }
            }
            Expression::BinaryOp(binop) => {
                matches!(binop.operator, BinaryOperator::Add)
                    && (self.is_array_like_expr(&binop.left)
                        || self.is_array_like_expr(&binop.right))
            }
            Expression::Call(call) => {
                // 方法调用返回数组的情况
                if let Some(obj) = &call.object {
                    if call.method.as_deref() == Some("push") {
                        return true; // push 返回新数组
                    }
                    self.is_array_like_expr(obj)
                } else if let Some(Some(ast_ret)) = self.func_return_types.get(&call.callee) {
                    matches!(ast_ret, Type::List(_))
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// 从 AST 表达式直接推断源级类型（不依赖已生成的 IR）
    fn infer_source_type_from_expr(&self, expr: &Expression) -> Option<Type> {
        match expr {
            Expression::Literal { value, .. } => {
                match value {
                    Literal::Number(_) => Some(Type::Number),
                    Literal::Boolean(_) => Some(Type::Boolean),
                    Literal::String(_) => Some(Type::String),
                    Literal::Null => Some(Type::Null),
                    Literal::List(elems) => {
                        // 推断元素类型
                        if let Some(first) = elems.first() {
                            let elem_ty =
                                self.infer_source_type_from_expr(first).unwrap_or(Type::Any);
                            Some(Type::List(Box::new(elem_ty)))
                        } else {
                            Some(Type::List(Box::new(Type::Any)))
                        }
                    }
                    Literal::Object(fields) => {
                        let field_types: Vec<(String, Type)> = fields
                            .iter()
                            .filter_map(|f| match f {
                                ObjectField::Field { name, value } => {
                                    let ty = self
                                        .infer_source_type_from_expr(value)
                                        .unwrap_or(Type::Any);
                                    Some((name.clone(), ty))
                                }
                                _ => None,
                            })
                            .collect();
                        Some(Type::ObjectType(field_types))
                    }
                }
            }
            Expression::BinaryOp(binop) => {
                // + on two lists = list concat
                if matches!(binop.operator, BinaryOperator::Add) {
                    let left_ty = self.infer_source_type_from_expr(&binop.left);
                    let right_ty = self.infer_source_type_from_expr(&binop.right);
                    if let (Some(Type::List(_)), _) = (&left_ty, &right_ty) {
                        return left_ty;
                    }
                    if let (_, Some(Type::List(_))) = (&left_ty, &right_ty) {
                        return right_ty;
                    }
                    // 字符串拼接（模板字面量 desugar 成 +）结果是字符串
                    if left_ty == Some(Type::String) || right_ty == Some(Type::String) {
                        return Some(Type::String);
                    }
                }
                None
            }
            Expression::Identifier { name, .. } => self.source_types.get(name).cloned(),
            Expression::Call(call) => {
                if call.object.is_none() {
                    if call.callee == "str" {
                        return Some(Type::String);
                    }
                    // Math::/Time:: 内建：按 lowering 表的 tag 映射返回类型
                    if let Some((_, _, tag)) = Self::intrinsic_lowering(&call.callee) {
                        return Some(if tag == 0 { Type::String } else { Type::Number });
                    }
                    // 结构体构造：Point(x: 1) -> Point
                    if self.structs.contains_key(&call.callee) {
                        return Some(Type::Named(call.callee.clone(), Vec::new()));
                    }
                }
                // 方法调用用 `Type::method` 作键；否则用函数名
                let key = if let Some(recv) = &call.object
                    && let Some(recv_ty) = self.struct_name_of_expr(recv)
                    && let Some(method) = call.method.as_deref()
                {
                    format!("{}::{}", recv_ty, method)
                } else {
                    call.callee.clone()
                };
                self.func_return_types.get(&key).cloned().flatten()
            }
            Expression::If(if_expr) => {
                // 取 then 分支的类型
                if let Some(Statement::Expr(e)) = if_expr.then_branch.statements.last() {
                    return self.infer_source_type_from_expr(&e.expr);
                }
                None
            }
            Expression::Member(member) => {
                // 结构体字段类型
                if let Some(recv_ty) = self.struct_name_of_expr(&member.object)
                    && let Some(fields) = self.structs.get(&recv_ty)
                    && let Some(field) = fields.iter().find(|f| f.name == member.property)
                {
                    return Some(field.type_.clone());
                }
                None
            }
            // `std::math` 常量（PI/E/TAU/INFINITY/NAN）都是 float
            Expression::EnumRef(e) if e.enum_name == "std::math" => Some(Type::Number),
            _ => None,
        }
    }

    /// 确定表达式的类型标签（用于数组/对象元素存储）
    /// tag: 0=string, 1=int, 2=float, 3=array, 4=object, 5=bool, 6=null
    fn determine_element_tag(&self, expr: &Expression) -> i64 {
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
            Expression::Identifier { name, .. } => {
                // match 绑定：payload 标签运行时才知道
                if self.is_dyn_name(name) {
                    return -1;
                }
                // `use std::math::*` 导入的裸名常量走 F64 槽（tag=2）
                if Self::bare_math_const(name).is_some() && !self.locals.contains_key(name) {
                    return 2;
                }
                // 装箱变量按逻辑类型打标签（cell 本地槽是 Pointer，不能直接用）
                if let Some(logical) = self.boxed_logical.get(name) {
                    return match logical {
                        IrType::F64 => 2,
                        IrType::I64 => 1,
                        IrType::Bool => 5,
                        IrType::Pointer => -1,
                        IrType::Void => 6,
                    };
                }
                // 优先从 source_types 查找
                if let Some(src_ty) = self.source_types.get(name) {
                    match src_ty {
                        Type::String => 0,
                        Type::Number => {
                            // number 绑定若以 F64 槽承载（小数字面量），值是位模式，按 tag=2 打印
                            if let Some(local_id) = self.locals.get(name)
                                && let Some(func_idx) = self.current_func
                                && self.module.functions[func_idx]
                                    .locals
                                    .iter()
                                    .any(|(id, ty)| id == local_id && matches!(ty, IrType::F64))
                            {
                                2
                            } else {
                                1
                            }
                        }
                        Type::Boolean => 5,
                        Type::Null => 6,
                        Type::List(_) => 3,
                        Type::Object | Type::ObjectType(_) => 4,
                        Type::Named(_, _) => -1, // 泛型参数，运行时自动检测
                        _ => 1,
                    }
                } else if let Some(local_id) = self.locals.get(name) {
                    // 从 IR 本地变量类型推断
                    if let Some(func_idx) = self.current_func {
                        if let Some((_, ir_ty)) = self.module.functions[func_idx]
                            .locals
                            .iter()
                            .find(|(id, _)| id == local_id)
                        {
                            match ir_ty {
                                IrType::I64 => 1,
                                IrType::F64 => 2,
                                IrType::Bool => 5,
                                IrType::Pointer => -1, // 泛型指针类型，运行时自动检测
                                IrType::Void => 6,
                            }
                        } else {
                            1
                        }
                    } else {
                        1
                    }
                } else {
                    1
                }
            }
            Expression::BinaryOp(binop) => {
                match &binop.operator {
                    BinaryOperator::Add => {
                        // 检查是否是数组拼接
                        if self.is_array_like_expr(&binop.left)
                            || self.is_array_like_expr(&binop.right)
                        {
                            3 // array
                        } else if self.is_string_like(&binop.left)
                            || self.is_string_like(&binop.right)
                        {
                            0 // string（模板/字符串拼接）
                        } else if self.expr_is_dyn(&binop.left)
                            || self.expr_is_dyn(&binop.right)
                            || self.has_float_operand(&binop.left)
                            || self.has_float_operand(&binop.right)
                        {
                            2
                        } else {
                            1
                        }
                    }
                    BinaryOperator::Sub
                    | BinaryOperator::Mul
                    | BinaryOperator::Div
                    | BinaryOperator::Mod
                    | BinaryOperator::Pow
                    | BinaryOperator::BitAnd
                    | BinaryOperator::BitOr
                    | BinaryOperator::Xor
                    | BinaryOperator::Shl
                    | BinaryOperator::Shr => {
                        // 动态绑定参与的数值运算走 f64，结果按位模式承载
                        if self.expr_is_dyn(&binop.left)
                            || self.expr_is_dyn(&binop.right)
                            || self.has_float_operand(&binop.left)
                            || self.has_float_operand(&binop.right)
                        {
                            2
                        } else {
                            1
                        }
                    }
                    BinaryOperator::Eq
                    | BinaryOperator::Neq
                    | BinaryOperator::Lt
                    | BinaryOperator::Gt
                    | BinaryOperator::Lte
                    | BinaryOperator::Gte
                    | BinaryOperator::And
                    | BinaryOperator::Or => 5,
                }
            }
            Expression::Unary(unop) => match &unop.operator {
                UnaryOperator::Not => 5,
                UnaryOperator::Neg | UnaryOperator::BitNot => {
                    self.determine_element_tag(&unop.operand)
                }
                UnaryOperator::Move | UnaryOperator::Copy => {
                    self.determine_element_tag(&unop.operand)
                }
            },
            Expression::Call(call) => {
                // 经局部变量的间接调用：返回标签运行时才知道
                if call.object.is_none() && self.locals.contains_key(&call.callee) {
                    return -1;
                }
                // 方法调用用 `Type::method` 作键查找返回类型注解
                let key = if call.object.is_some() {
                    let method = call.method.as_deref().unwrap_or(&call.callee);
                    match call
                        .object
                        .as_ref()
                        .and_then(|o| self.struct_name_of_expr(o))
                    {
                        Some(ty) => format!("{}::{}", ty, method),
                        None => method.to_string(),
                    }
                } else {
                    call.callee.clone()
                };
                // str() 产出字符串；Math::Time:: 内建与 float 位模式返回按表打 tag
                if key == "str" {
                    return 0;
                }
                if let Some((_, _, tag)) = Self::intrinsic_lowering(&key) {
                    return tag;
                }
                if self.float_bit_returns.contains(&key) {
                    return 2;
                }
                // `Enum::Variant(...)` 构造值
                if call.object.is_none()
                    && let Some((enum_name, variant)) = key.split_once("::")
                    && self.enum_variant_index(enum_name, variant).is_some()
                {
                    return 7;
                }
                // 优先使用 AST 返回类型注解推断 tag
                if let Some(Some(ast_ret)) = self.func_return_types.get(&key) {
                    match ast_ret {
                        Type::String => 0,
                        // number 的返回可能是 f64 位模式（float 结果在返回边界
                        // 按位打包），交给运行时自动检测
                        Type::Number => -1,
                        Type::Boolean => 5,
                        Type::Null => 6,
                        Type::List(_) => 3,
                        Type::Object | Type::ObjectType(_) => 4,
                        Type::Named(_, _) => {
                            // 泛型参数：无法在编译期确定具体类型，使用运行时自动检测
                            -1
                        }
                        _ => 1,
                    }
                } else {
                    // 退化到 IR 类型
                    if let Some(func_idx) = self.module.functions.iter().position(|f| f.name == key)
                    {
                        match self.module.functions[func_idx].return_type {
                            IrType::I64 => 1,
                            IrType::F64 => 2,
                            IrType::Bool => 5,
                            IrType::Void => 6,
                            IrType::Pointer => {
                                // 指针类型可能是 string/array/object/泛型，使用运行时自动检测
                                -1
                            }
                        }
                    } else {
                        1
                    }
                }
            }
            // `std::math` 常量（PI/E/TAU/INFINITY/NAN）都是 float
            Expression::EnumRef(e) if e.enum_name == "std::math" => 2,
            // 其余枚举值（含无 payload 的变体引用）
            Expression::EnumRef(_) => 7,
            // match 表达式：分支结果各异，运行时检测
            Expression::Match(_) => -1,
            // 函数值（闭包/箭头字面量）：tag=8
            Expression::FnExpr(_) | Expression::Arrow(_) => 8,
            // 经任意表达式的函数值调用：返回标签未知
            Expression::CallValue(_) => -1,
            Expression::Nullish(nullish) => self.determine_element_tag(&nullish.right),
            Expression::Index(_) => -1, // 数组元素类型未知，运行时自动检测
            Expression::Member(member) => {
                // 结构体字段类型已知
                if let Some(recv_ty) = self.struct_name_of_expr(&member.object)
                    && let Some(fields) = self.structs.get(&recv_ty)
                    && let Some(field) = fields.iter().find(|f| f.name == member.property)
                {
                    return match &field.type_ {
                        Type::String => 0,
                        Type::Number => 1,
                        Type::Boolean => 5,
                        Type::Null => 6,
                        Type::List(_) => 3,
                        Type::Object | Type::ObjectType(_) => 4,
                        // 可选字段可能是 null，运行时自动检测
                        _ => -1,
                    };
                }
                -1 // 对象字段类型未知，运行时自动检测
            }
            _ => 1,
        }
    }

    /// 名字是否是 match 绑定（payload 标签运行时才知道）
    fn is_dyn_name(&self, name: &str) -> bool {
        self.dyn_tag_of.contains_key(name)
    }

    fn expr_is_dyn(&self, expr: &Expression) -> bool {
        matches!(expr, Expression::Identifier { name, .. } if self.is_dyn_name(name))
    }

    /// 动态绑定的操作数经 `xulo_as_f64` 转成 f64 槽；tag 取绑定时摘出的
    /// companion 槽（构造侧记录的是真实 payload 标签）
    fn as_f64_operand(&mut self, operand: Operand, name: &str) -> Result<Operand, XuloError> {
        let Some(tag_op) = self.dyn_tag_of.get(name).cloned() else {
            return Ok(operand);
        };
        let func_idx = self.current_func.unwrap();
        let local = self.module.functions[func_idx].add_local(IrType::F64);
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: Some(local),
                func: RuntimeFn::AsF64,
                args: vec![operand, tag_op],
            });
        Ok(Operand::Local(local))
    }

    /// f64 值在按位模式进 I64/Pointer 槽（return / match 结果）前注册位模式，
    /// 打印端的自动检测（tag=-1）才能还原成小数
    fn emit_register_float(&mut self, operand: Operand) {
        let func_idx = self.current_func.unwrap();
        self.module.functions[func_idx]
            .instructions
            .push(Instruction::RuntimeCall {
                dst: None,
                func: RuntimeFn::RegisterFloat,
                args: vec![operand],
            });
    }

    /// `Enum::Variant` 的变体序号（声明序）
    fn enum_variant_index(&self, enum_name: &str, variant: &str) -> Option<usize> {
        self.enums
            .get(enum_name)
            .and_then(|vs| vs.iter().position(|v| v == variant))
    }

    /// 枚举构造：AllocEnum + 逐 payload 槽 EnumSet（值 + 静态推断的标签）
    fn generate_enum_construct(
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
        let type_idx = self.module.add_string(enum_name.to_string());
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

    fn has_float_operand(&self, expr: &Expression) -> bool {
        match expr {
            Expression::Literal { value, .. } => {
                matches!(value, Literal::Number(n) if n.fract() != 0.0)
            }
            Expression::Identifier { name, .. } => {
                // F64 槽承载的变量（装箱逻辑类型或本地槽）参与算术走浮点
                if self.boxed_logical.get(name) == Some(&IrType::F64) {
                    return true;
                }
                if let Some(local_id) = self.locals.get(name)
                    && let Some(func_idx) = self.current_func
                {
                    return self.module.functions[func_idx]
                        .locals
                        .iter()
                        .any(|(id, ty)| id == local_id && matches!(ty, IrType::F64));
                }
                false
            }
            Expression::Unary(unop) => self.has_float_operand(&unop.operand),
            Expression::BinaryOp(binop) => {
                self.has_float_operand(&binop.left) || self.has_float_operand(&binop.right)
            }
            _ => false,
        }
    }

    /// 表达式是否是字符串（用于 `+` 拼接与 str() 直通的 tag 判断）
    fn is_string_like(&self, expr: &Expression) -> bool {
        if matches!(self.infer_source_type_from_expr(expr), Some(Type::String)) {
            return true;
        }
        // str() 的返回值
        matches!(expr, Expression::Call(call)
            if call.object.is_none() && call.callee == "str")
    }

    fn generate_return(&mut self, r: &ReturnStmt) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();

        if let Some(value) = &r.value {
            let operand = self.generate_expression(value)?;
            if self.determine_element_tag(value) == 2 {
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

        Ok(())
    }

    fn generate_expression(&mut self, expr: &Expression) -> Result<Operand, XuloError> {
        match expr {
            Expression::Literal { value, .. } => self.generate_literal(value),
            Expression::Identifier { name, .. } => {
                if let Some(local) = self.locals.get(name).copied() {
                    if self.boxed_names.contains(name) {
                        // 装箱变量：从 cell 读出（f64 位模式走专用读回）
                        let logical = self
                            .boxed_logical
                            .get(name)
                            .cloned()
                            .unwrap_or(IrType::I64);
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
                } else if self.module.functions.iter().any(|f| f.name == *name) {
                    // 顶层函数引用作函数值：经 thunk 适配成闭包
                    self.generate_func_value(name)
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
            // 无 payload 的枚举变体引用：`Direction::East`
            Expression::EnumRef(e) => self.generate_enum_construct(&e.enum_name, &e.variant, &[]),
            Expression::Match(m) => self.generate_match(m),
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

    fn generate_literal(&mut self, lit: &Literal) -> Result<Operand, XuloError> {
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

    fn generate_binary_op(&mut self, binop: &BinaryOp) -> Result<Operand, XuloError> {
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
            BinaryOperator::Mod | BinaryOperator::Pow => IrType::I64,
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

    /// IR 操作数的槽类型（Neg 等按类型选择本地变量类型与指令）
    fn operand_slot_type(&self, op: &Operand) -> IrType {
        match op {
            Operand::Const(IrValue::F64(_)) => IrType::F64,
            Operand::Const(IrValue::String(_)) | Operand::Const(IrValue::Null) => IrType::Pointer,
            Operand::Const(_) => IrType::I64,
            Operand::Local(id) => {
                let Some(func_idx) = self.current_func else {
                    return IrType::I64;
                };
                self.module.functions[func_idx]
                    .locals
                    .iter()
                    .find(|(lid, _)| lid == id)
                    .map(|(_, ty)| ty.clone())
                    .unwrap_or(IrType::I64)
            }
        }
    }

    fn generate_unary(&mut self, unary: &UnaryOp) -> Result<Operand, XuloError> {
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

    fn generate_call(&mut self, call: &Call) -> Result<Operand, XuloError> {
        let func_idx = self.current_func.unwrap();

        // 多参数 print：逐参数 stringify 后用空格拼接（与解释器 native_print 一致）
        if call.object.is_none()
            && matches!(call.callee.as_str(), "print" | "println")
            && call.arguments.len() > 1
        {
            return self.generate_print_multi(call);
        }

        // 结构体构造：Point(x: 1) -> NewObject
        if call.object.is_none()
            && call.method.is_none()
            && let Some(fields) = self.structs.get(&call.callee).cloned()
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

        for arg in &call.arguments {
            args.push(self.generate_expression(&arg.value)?);
        }

        // str(字符串) 直通：模板 desugar 会对字符串也包一层 str()
        if call.object.is_none()
            && call.callee == "str"
            && args.len() == 1
            && self.determine_element_tag(&call.arguments[0].value) == 0
        {
            return Ok(args.swap_remove(0));
        }

        // `std::math::` / `std::time::` 命名空间内建（语义层已按
        // builtin_member 校验成员存在；裸名调用已被 apply_std_calls 改写为
        // 限定形式）
        if call.object.is_none()
            && (call.callee.starts_with("std::math::") || call.callee.starts_with("std::time::"))
        {
            return self.generate_intrinsic_call(&call.callee, &args);
        }

        // 检查是否是内置函数或内置方法
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

    /// 顶层函数引用作函数值：生成（或复用）thunk 闭包，
    /// 把直接调用协议适配成 CallIndirect 的 `(env, a0..a7)` 协议
    fn generate_func_value(&mut self, name: &str) -> Result<Operand, XuloError> {
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
    fn get_or_create_thunk(&mut self, target_name: &str) -> Result<FuncId, XuloError> {
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
        let call_args: Vec<Operand> = (0..n_params).map(|i| Operand::Local(LocalId(i + 1))).collect();
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
        func.instructions.push(Instruction::Return(dst.map(Operand::Local)));
        let fid = self.module.add_function(func);
        self.func_thunks.insert(target_name.to_string(), fid);
        Ok(fid)
    }

    /// 内嵌 `fn` 语句：在声明点生成闭包值（捕获当时作用域的自由变量）
    fn generate_inner_fn(&mut self, f: &FnDef) -> Result<(), XuloError> {
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

        self.generate_closure_body(func_id.0, &f.params, &f.body, &capture, f.return_type.as_ref())?;
        Ok(())
    }

    /// 闭包/箭头字面量求值：建 env + MakeClosure（仍在外层作用域），
    /// 然后生成体函数
    fn generate_closure_value(
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
        let ret_ty = self.type_to_ir(return_type);
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
    fn emit_closure_creation(
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
                XuloError::new(
                    ErrorKind::Semantic,
                    format!("undefined variable: {}", name),
                )
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
    fn generate_closure_body(
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

        // 体语句（尾表达式若作为返回值则延后、只生成一次）
        let ret_type = self.type_to_ir(return_type);
        let last_stmt_is_return = matches!(body.statements.last(), Some(Statement::Return(_)));
        let skip_last = !ret_type.is_void()
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
                if !ret_type.is_void() {
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
    fn swap_scope(&mut self) -> ScopeSnapshot {
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
    fn restore_scope(&mut self, snap: ScopeSnapshot) {
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

    /// 经函数值间接调用：实参超过 8 个无法塞进闭包入口协议
    fn generate_call_indirect(
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
    fn resolve_callee(&self, call: &Call) -> Option<usize> {
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
            self.module
                .functions
                .iter()
                .position(|f| f.name == call.callee)
        }
    }

    /// 从表达式取结构体（Named）类型名
    fn struct_name_of_expr(&self, expr: &Expression) -> Option<String> {
        match self.infer_source_type_from_expr(expr) {
            Some(Type::Named(name, _)) => Some(name),
            _ => None,
        }
    }

    /// 结构体构造：按字段顺序 NewObject；命名参数、位置参数、默认值、缺省 null
    fn generate_struct_ctor(
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

    /// Math/Time 命名空间内建的 lowering 表（对齐语义 `builtin_member`，spec
    /// `builtins/intrinsic-functions.md`）：返回 (运行时符号, 结果槽类型, 打印 tag)。
    /// float 组返回 f64，但 xulo 的 number 在 IR 层是整数域——按 F64 位模式放进
    /// I64 槽（打印端 tag=2 还原小数）；toFixed 返回字符串；Time 读数返回整数。
    /// `use std::math::*` 导入的裸名常量在 locals 缺席时的 F64 值。
    fn bare_math_const(name: &str) -> Option<f64> {
        match name {
            "PI" => Some(std::f64::consts::PI),
            "E" => Some(std::f64::consts::E),
            "TAU" => Some(std::f64::consts::TAU),
            "INFINITY" => Some(f64::INFINITY),
            "NAN" => Some(f64::NAN),
            _ => None,
        }
    }

    fn intrinsic_lowering(callee: &str) -> Option<(&'static str, IrType, i64)> {
        // `std::math::abs` / `std::time::now` 归一到表键 `Math::abs` 等
        let normalized: std::borrow::Cow<str> =
            if let Some(rest) = callee.strip_prefix("std::math::") {
                std::borrow::Cow::Owned(format!("Math::{rest}"))
            } else if let Some(rest) = callee.strip_prefix("std::time::") {
                std::borrow::Cow::Owned(format!("Time::{rest}"))
            } else {
                std::borrow::Cow::Borrowed(callee)
            };
        let lowering = match normalized.as_ref() {
            "Math::abs" => ("xulo_math_abs", IrType::I64, 2),
            "Math::min" => ("xulo_math_min", IrType::I64, 2),
            "Math::max" => ("xulo_math_max", IrType::I64, 2),
            "Math::floor" => ("xulo_math_floor", IrType::I64, 2),
            "Math::ceil" => ("xulo_math_ceil", IrType::I64, 2),
            "Math::round" => ("xulo_math_round", IrType::I64, 2),
            "Math::trunc" => ("xulo_math_trunc", IrType::I64, 2),
            "Math::sqrt" => ("xulo_math_sqrt", IrType::I64, 2),
            "Math::cbrt" => ("xulo_math_cbrt", IrType::I64, 2),
            "Math::pow" => ("xulo_math_pow", IrType::I64, 2),
            "Math::sin" => ("xulo_math_sin", IrType::I64, 2),
            "Math::cos" => ("xulo_math_cos", IrType::I64, 2),
            "Math::tan" => ("xulo_math_tan", IrType::I64, 2),
            "Math::asin" => ("xulo_math_asin", IrType::I64, 2),
            "Math::acos" => ("xulo_math_acos", IrType::I64, 2),
            "Math::atan" => ("xulo_math_atan", IrType::I64, 2),
            "Math::atan2" => ("xulo_math_atan2", IrType::I64, 2),
            "Math::log" => ("xulo_math_log", IrType::I64, 2),
            "Math::log2" => ("xulo_math_log2", IrType::I64, 2),
            "Math::log10" => ("xulo_math_log10", IrType::I64, 2),
            "Math::exp" => ("xulo_math_exp", IrType::I64, 2),
            "Math::toFixed" => ("xulo_math_to_fixed", IrType::Pointer, 0),
            "Time::now" => ("xulo_time_now", IrType::I64, 1),
            "Time::now_nanos" => ("xulo_time_now_nanos", IrType::I64, 1),
            "Time::monotonic" => ("xulo_time_monotonic", IrType::I64, 1),
            "Time::monotonic_nanos" => ("xulo_time_monotonic_nanos", IrType::I64, 1),
            _ => return None,
        };
        Some(lowering)
    }

    /// Math::/Time:: 内建调用：RuntimeCall 落到对应运行时符号
    fn generate_intrinsic_call(
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
    fn generate_print_multi(&mut self, call: &Call) -> Result<Operand, XuloError> {
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
    fn generate_nullish(&mut self, nullish: &NullishExpr) -> Result<Operand, XuloError> {
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

    fn generate_if(&mut self, if_expr: &IfExpr) -> Result<(), XuloError> {
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
        for stmt in &if_expr.then_branch.statements {
            self.generate_statement(stmt)?;
        }
        let then_has_return = matches!(
            self.module.functions[func_idx].instructions.last(),
            Some(Instruction::Return(_))
        );
        if !then_has_return {
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Jump(end_label));
        }

        self.module.functions[func_idx]
            .instructions
            .push(Instruction::Label(else_label));
        if let Some(else_branch) = &if_expr.else_branch {
            for stmt in &else_branch.statements {
                self.generate_statement(stmt)?;
            }
        }
        let else_has_return = matches!(
            self.module.functions[func_idx].instructions.last(),
            Some(Instruction::Return(_))
        );
        if !else_has_return {
            self.module.functions[func_idx]
                .instructions
                .push(Instruction::Jump(end_label));
        }

        // Only emit end_label if at least one branch jumps to it
        if !then_has_return || !else_has_return {
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

    fn generate_if_expr(&mut self, if_expr: &IfExpr) -> Result<Operand, XuloError> {
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
    fn generate_match(&mut self, m: &MatchExpr) -> Result<Operand, XuloError> {
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
                            let cell =
                                self.module.functions[func_idx].add_local(IrType::Pointer);
                            self.module.functions[func_idx].instructions.push(
                                Instruction::RuntimeCall {
                                    dst: Some(cell),
                                    func: RuntimeFn::AllocCell,
                                    args: vec![Operand::Local(raw)],
                                },
                            );
                            self.locals.insert(bind.clone(), cell);
                            self.boxed_logical
                                .insert(bind.clone(), IrType::Pointer);
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
                MatchPattern::Wildcard => {
                    self.generate_match_arm_body(arm, result_local)?;
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Jump(end_label));
                }
                MatchPattern::Binding { name, .. } => {
                    if self.boxed_names.contains(name) {
                        let cell = self.module.functions[func_idx].add_local(IrType::Pointer);
                        self.module.functions[func_idx]
                            .instructions
                            .push(Instruction::RuntimeCall {
                                dst: Some(cell),
                                func: RuntimeFn::AllocCell,
                                args: vec![scrut_op.clone()],
                            });
                        self.locals.insert(name.clone(), cell);
                        self.boxed_logical.insert(name.clone(), IrType::Pointer);
                    } else {
                        self.locals.insert(name.clone(), scrut_local);
                    }
                    self.dyn_tag_of
                        .insert(name.clone(), Operand::Const(IrValue::I64(7)));
                    self.generate_match_arm_body(arm, result_local)?;
                    self.module.functions[func_idx]
                        .instructions
                        .push(Instruction::Jump(end_label));
                }
                _ => {
                    return Err(XuloError::new(
                        ErrorKind::Semantic,
                        "AOT codegen supports only enum, `_`, and binding match patterns"
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

    /// 枚举变体的 tag 分支：`EnumVariant(scrut) == idx` 为真走本臂，否则下一臂
    fn emit_enum_tag_branch(
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
    fn generate_match_arm_body(
        &mut self,
        arm: &MatchArm,
        result_local: LocalId,
    ) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();
        let value_expr = match &arm.value {
            MatchBody::Expr(e) => Some(e),
            MatchBody::Block(b) => b.statements.last().and_then(|s| match s {
                Statement::Expr(e) => Some(&e.expr),
                _ => None,
            }),
        };
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
        }
        Ok(())
    }

    fn generate_for(&mut self, for_stmt: &ForStmt) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();

        // 创建循环变量（被捕获时本地槽持有 cell；每次迭代换新 cell，
        // 同一轮迭代里创建的闭包共享该轮的值）
        let boxed = self.boxed_names.contains(&for_stmt.iter_var);
        let iter_local = self.module.functions[func_idx]
            .add_local(if boxed { IrType::Pointer } else { IrType::I64 });
        self.locals.insert(for_stmt.iter_var.clone(), iter_local);
        if boxed {
            let logical = if matches!(for_stmt.iterable, Expression::Range(_)) {
                IrType::I64
            } else if let Some(Type::List(inner)) =
                self.infer_source_type_from_expr(&for_stmt.iterable)
            {
                self.type_to_ir(Some(&inner))
            } else {
                IrType::Pointer
            };
            self.boxed_logical.insert(for_stmt.iter_var.clone(), logical);
        }

        // 推断迭代变量的源级类型
        if let Some(Type::List(inner)) = self.infer_source_type_from_expr(&for_stmt.iterable) {
            self.source_types.insert(for_stmt.iter_var.clone(), *inner);
        }

        let start_label = self.module.functions[func_idx].add_label();
        let body_label = self.module.functions[func_idx].add_label();
        let end_label = self.module.functions[func_idx].add_label();
        let continue_label = self.module.functions[func_idx].add_label();

        self.loop_stack.push(LoopContext {
            break_label: end_label,
            continue_label,
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
            for stmt in &for_stmt.body.statements {
                self.generate_statement(stmt)?;
            }

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
            let array_operand = self.generate_expression(&for_stmt.iterable)?;
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
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::RuntimeCall {
                        dst: Some(iter_local),
                        func: RuntimeFn::AllocCell,
                        args: vec![Operand::Local(raw)],
                    });
            } else {
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::GetIndex {
                        dst: iter_local,
                        array: Operand::Local(array_local),
                        index: Operand::Local(index_local),
                    });
            }
            for stmt in &for_stmt.body.statements {
                self.generate_statement(stmt)?;
            }

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

    fn generate_while(&mut self, while_stmt: &WhileStmt) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();

        let start_label = self.module.functions[func_idx].add_label();
        let body_label = self.module.functions[func_idx].add_label();
        let end_label = self.module.functions[func_idx].add_label();
        let continue_label = self.module.functions[func_idx].add_label();

        self.loop_stack.push(LoopContext {
            break_label: end_label,
            continue_label,
        });

        // 循环开始
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
        for stmt in &while_stmt.body.statements {
            self.generate_statement(stmt)?;
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

    fn generate_assign(&mut self, assign: &AssignStmt) -> Result<(), XuloError> {
        let func_idx = self.current_func.unwrap();
        let value = self.generate_expression(&assign.value)?;

        match &assign.target {
            AssignTarget::Name(name) => {
                if let Some(local) = self.locals.get(name).copied() {
                    if self.boxed_names.contains(name) {
                        // 装箱变量：写入既有 cell（别名的闭包共享同一 cell）
                        self.module.functions[func_idx]
                            .instructions
                            .push(Instruction::RuntimeCall {
                                dst: None,
                                func: RuntimeFn::CellSet,
                                args: vec![Operand::Local(local), value],
                            });
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
                let tag = self.determine_element_tag(&assign.value);
                self.module.functions[func_idx]
                    .instructions
                    .push(Instruction::SetField {
                        object: obj_operand,
                        field: field.clone(),
                        value,
                        tag,
                    });
            }
            AssignTarget::Index(obj, idx) => {
                let obj_operand = self.generate_expression(obj)?;
                let idx_operand = self.generate_expression(idx)?;
                let tag = self.determine_element_tag(&assign.value);
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

        Ok(())
    }

    fn generate_member_access(&mut self, member: &MemberAccess) -> Result<Operand, XuloError> {
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

    fn generate_index(&mut self, index: &IndexExpr) -> Result<Operand, XuloError> {
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

    fn type_to_ir(&self, ty: Option<&Type>) -> IrType {
        match ty {
            None => IrType::Void,
            Some(t) => match t {
                Type::Boolean => IrType::Bool,
                Type::String
                | Type::Null
                | Type::Object
                | Type::List(_)
                | Type::Named(_, _)
                | Type::Literal(_)
                | Type::Optional(_)
                | Type::Union(_)
                | Type::Intersection(_)
                | Type::ObjectType(_)
                | Type::FnSig { .. }
                | Type::Async(_)
                | Type::Any
                | Type::Tuple(_) => IrType::Pointer,
                Type::Number => IrType::I64,
            },
        }
    }
}

/// 从 AST 生成 IR 模块
pub fn generate_ir(program: &Program) -> Result<IrModule, XuloError> {
    let mut generator = IrGenerator::new();
    generator.generate(program)
}

/// 有序去重插入（自由变量/绑定收集保持首次出现顺序）
fn push_unique(list: &mut Vec<String>, name: &str) {
    if !list.iter().any(|n| n == name) {
        list.push(name.to_string());
    }
}
