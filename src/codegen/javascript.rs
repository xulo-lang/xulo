use crate::ast::{
    AssignStmt, BindingPattern, BinaryOperator, Block, BinaryOp, Call, CallValue, ComponentStmt,
    EnumDef, Expression, FnDef, ForStmt, IfExpr, LetBinding, Literal, ObjectField, Program,
    Statement, Type, UiElement, WhileStmt,
};
use crate::error::XuloError;
const INDENT: &str = "    ";

/// The minimal reactive runtime, emitted once when any `@State`/`@Store`/
/// `@Effect`/`@Environment`/`Component` feature is used. Provides signals
/// (`__signal`), effects (`__effect`), a component render wrapper
/// (`__component`), and environment lookup (`__env`).
const REACTIVE_RUNTIME: &str = r#"const __runtime = (() => {
    let current = null;
    function signal(v) {
        let value = v;
        const subs = new Set();
        return {
            get() { if (current) current.add(subs); return value; },
            set(nv) { value = nv; const s = [...subs]; s.forEach(f => f()); }
        };
    }
    function effect(fn) {
        let cleanup = null;
        function run() {
            if (cleanup) { const c = cleanup; cleanup = null; c(); }
            const prev = current;
            const track = new Set();
            current = track;
            let out;
            try { out = fn(); } finally { current = prev; }
            track.forEach(s => s.add(run));
            if (typeof out === "function") cleanup = out;
        }
        run();
    }
    function component(render) {
        let cleanup = null;
        let last;
        function run() {
            if (cleanup) { const c = cleanup; cleanup = null; c(); }
            const prev = current;
            const track = new Set();
            current = track;
            try { last = render(); } finally { current = prev; }
            track.forEach(s => s.add(run));
        }
        run();
        return { get value() { return last; }, rerender: run };
    }
    function env(name) { return (globalThis.__xulo_env || {})[name]; }
    return { signal, effect, component, env };
})();
const __signal = __runtime.signal;
const __effect = __runtime.effect;
const __component = __runtime.component;
const __env = __runtime.env;
"#;

/// Emits modern JavaScript (ES Module) for a Xulo program.
pub struct Javascript {
    out: String,
    indent: usize,
    /// `function name -> declared parameter order` (used to reorder named
    /// call arguments).
    fn_params: std::collections::HashMap<String, Vec<String>>,
    /// Stack of scopes of `@State` signal names (used to rewrite reads into
    /// `.get()` and writes into `.set()`).
    signals: Vec<std::collections::HashSet<String>>,
    /// Whether any reactive feature was used (triggers the runtime preamble).
    used_reactive: bool,
}

impl Default for Javascript {
    fn default() -> Self {
        Self::new()
    }
}

impl Javascript {
    pub fn new() -> Self {
        Self {
            out: String::new(),
            indent: 0,
            fn_params: std::collections::HashMap::new(),
            signals: vec![std::collections::HashSet::new()],
            used_reactive: false,
        }
    }

    /// A fresh codegen sharing this one's parameter/signal context (used for
    /// sub-expressions and closure bodies).
    fn child(&self) -> Javascript {
        Javascript {
            out: String::new(),
            indent: self.indent,
            fn_params: self.fn_params.clone(),
            signals: self.signals.clone(),
            used_reactive: false,
        }
    }

    fn mark_reactive(&mut self) {
        self.used_reactive = true;
    }

    fn is_signal(&self, name: &str) -> bool {
        self.signals.iter().rev().any(|s| s.contains(name))
    }

    fn register_signal(&mut self, name: String) {
        self.signals.last_mut().expect("signal scope").insert(name);
    }

    fn push_signal_scope(&mut self) {
        self.signals.push(std::collections::HashSet::new());
    }

    fn pop_signal_scope(&mut self) {
        if self.signals.len() > 1 {
            self.signals.pop();
        }
    }

    /// Register the parameter order of an imported function so calls in this
    /// module that use named arguments can be reordered.
    pub fn register_fn_params(&mut self, name: String, params: Vec<String>) {
        self.fn_params.insert(name, params);
    }

    pub fn finish(self) -> String {
        if self.used_reactive {
            let mut out = String::new();
            out.push_str(REACTIVE_RUNTIME);
            out.push_str(&self.out);
            out
        } else {
            self.out
        }
    }

    fn pad(&self) -> String {
        INDENT.repeat(self.indent)
    }

    fn line(&mut self, text: &str) {
        if text.is_empty() {
            self.out.push('\n');
        } else {
            self.out.push_str(&self.pad());
            self.out.push_str(text);
            self.out.push('\n');
        }
    }

    pub fn program(&mut self, program: &Program) -> Result<(), XuloError> {
        let has_main = program
            .statements
            .iter()
            .any(|s| matches!(s, Statement::Fn(f) if f.name == "main"));

        for statement in &program.statements {
            if let Statement::Fn(f) = statement {
                self.fn_params.insert(
                    f.name.clone(),
                    f.params.iter().map(|p| p.name.clone()).collect(),
                );
            }
        }

        for statement in &program.statements {
            self.statement(statement)?;
            self.out.push('\n');
        }

        if has_main {
            if main_returns_component(program) {
                self.line("const __xulo_main = main();");
                self.line("if (typeof __xulo_mount === \"function\") __xulo_mount(__xulo_main);");
            } else {
                self.line("main();");
            }
        }
        Ok(())
    }

    /// Emit an ES-module wrapper for one file: registers every function's
    /// parameter order (for named arguments), then emits its statements. Does
    /// *not* append `main();` — the module loader decides that.
    pub fn emit_module_body(&mut self, program: &Program) -> Result<(), XuloError> {
        for statement in &program.statements {
            if let Statement::Fn(f) = statement {
                self.fn_params.insert(
                    f.name.clone(),
                    f.params.iter().map(|p| p.name.clone()).collect(),
                );
            }
            if let Statement::Export(export) = statement {
                self.register_export_fn_params(&export.item);
            }
        }
        for statement in &program.statements {
            self.statement(statement)?;
            self.out.push('\n');
        }
        Ok(())
    }

    fn register_export_fn_params(&mut self, item: &crate::ast::ExportItem) {
        match item {
            crate::ast::ExportItem::Fn(f) => {
                self.fn_params.insert(
                    f.name.clone(),
                    f.params.iter().map(|p| p.name.clone()).collect(),
                );
            }
            crate::ast::ExportItem::Default(inner) => self.register_export_fn_params(inner),
            _ => {}
        }
    }

    fn statement(&mut self, statement: &Statement) -> Result<(), XuloError> {
        match statement {
            Statement::Fn(f) => self.fn_def(f)?,
            Statement::Let(b) => self.let_binding(b)?,
            Statement::Return(r) => {
                let value = self.expr(&r.value)?;
                self.line(&format!("return {value};"));
            }
            Statement::For(f) => self.for_stmt(f)?,
            Statement::While(w) => self.while_stmt(w)?,
            Statement::Block(b) => {
                self.line("{");
                self.indent += 1;
                self.block_body(b)?;
                self.indent -= 1;
                self.line("}");
            }
            Statement::Expr(Expression::If(if_expr)) => self.if_stmt(if_expr)?,
            Statement::Expr(expr) => {
                let value = self.expr(expr)?;
                self.line(&format!("{value};"));
            }
            Statement::Assign(a) => self.assign_stmt(a)?,
            // Type aliases are erased at codegen time.
            Statement::TypeAlias(_) => {}
            Statement::Enum(e) => self.enum_def(e)?,
            Statement::Try(t) => self.try_stmt(t)?,
            Statement::Throw(expr) => {
                let value = self.expr(expr)?;
                self.line(&format!("throw {value};"));
            }
            // Imports are handled at the module level; exports emit their
            // underlying declaration (imports/export-rewrites are tied to the
            // bundler).
            Statement::Import(_) => {}
            Statement::Export(export) => self.export_item(&export.item)?,
            Statement::State(state) => self.state_stmt(&state.binding)?,
            Statement::Store(store) => self.store_stmt(store)?,
            Statement::Effect(effect) => self.effect_stmt(effect)?,
            Statement::Environment(env) => self.environment_stmt(env)?,
            Statement::Component(component) => self.component_stmt(component)?,
        }
        Ok(())
    }

    /// Emit the runtime part of an `export` statement: the declaration itself.
    /// `export { a, b }` and `export type ...` are erased here (re-exported
    /// names already exist as statements; types have no runtime value).
    fn export_item(&mut self, item: &crate::ast::ExportItem) -> Result<(), XuloError> {
        match item {
            crate::ast::ExportItem::Fn(f) => self.fn_def(f)?,
            crate::ast::ExportItem::Let(b) => self.let_binding(b)?,
            crate::ast::ExportItem::Enum(e) => self.enum_def(e)?,
            crate::ast::ExportItem::Type(_) | crate::ast::ExportItem::Names(_) => {}
            crate::ast::ExportItem::Default(inner) => self.export_item(inner)?,
        }
        Ok(())
    }

    fn fn_def(&mut self, f: &FnDef) -> Result<(), XuloError> {
        if matches!(&f.return_type, Some(Type::Named(n)) if n == "Component") {
            return self.component_fn_def(f);
        }
        let params = f
            .params
            .iter()
            .map(|p| {
                let base = p.name.clone();
                match &p.default {
                    Some(d) => {
                        let d = self.expr(d)?;
                        Ok::<_, XuloError>(format!("{base} = {d}"))
                    }
                    None => Ok(base),
                }
            })
            .collect::<Result<Vec<_>, XuloError>>()?
            .join(", ");
        let kw = if f.is_async { "async function" } else { "function" };
        self.line(&format!("{kw} {}({params}) {{", f.name));
        self.indent += 1;
        let stmts = &f.body.statements;
        // Implicit return (docs §6 / §21.2): for a function with a declared
        // return type, a trailing expression statement is its value.
        if f.return_type.is_some()
            && let Some(Statement::Expr(last)) = stmts.last()
        {
            let value = self.expr(last)?;
            for s in &stmts[..stmts.len() - 1] {
                self.statement(s)?;
            }
            self.line(&format!("return {value};"));
            self.indent -= 1;
            self.line("}");
            return Ok(());
        }
        self.block_body(&f.body)?;
        self.indent -= 1;
        self.line("}");
        Ok(())
    }

    fn let_binding(&mut self, b: &LetBinding) -> Result<(), XuloError> {
        let kw = if b.is_const { "const" } else { "let" };
        match &b.value {
            Some(value) => {
                let value = self.expr(value)?;
                self.line(&format!("{kw} {} = {value};", b.name));
            }
            None => self.line(&format!("{kw} {};", b.name)),
        }
        Ok(())
    }

    fn assign_stmt(&mut self, a: &AssignStmt) -> Result<(), XuloError> {
        let value = self.expr(&a.value)?;
        if self.is_signal(&a.name) {
            self.line(&format!("{}.set({value});", a.name));
        } else {
            self.line(&format!("{} = {value};", a.name));
        }
        Ok(())
    }

    /// `@State let x = v` -> `const x = __signal(v);` (reads/writes of `x` are
    /// rewritten to `.get()`/`.set()` elsewhere).
    fn state_stmt(&mut self, binding: &LetBinding) -> Result<(), XuloError> {
        self.mark_reactive();
        let init = match &binding.value {
            Some(value) => self.expr(value)?,
            None => "undefined".to_string(),
        };
        self.line(&format!("const {} = __signal({init});", binding.name));
        self.register_signal(binding.name.clone());
        Ok(())
    }

    /// `@Store const { a, b } = expr` -> `const { a, b } = expr;` (value binding).
    fn store_stmt(&mut self, store: &crate::ast::StoreStmt) -> Result<(), XuloError> {
        self.mark_reactive();
        let value = self.expr(&store.value)?;
        match &store.pattern {
            BindingPattern::Ident(name) => {
                self.line(&format!("const {name} = {value};"));
            }
            BindingPattern::Destructure(fields) => {
                let names = fields
                    .iter()
                    .map(|(name, alias)| match alias {
                        Some(a) => format!("{name}: {a}"),
                        None => name.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                self.line(&format!("const {{ {names} }} = {value};"));
            }
        }
        Ok(())
    }

    /// `@Effect fn() { ... } [, [deps]]` -> `__effect(function(){...}, deps);`.
    fn effect_stmt(&mut self, effect: &crate::ast::EffectStmt) -> Result<(), XuloError> {
        self.mark_reactive();
        let closure = self.fn_expr(&effect.closure)?;
        let deps = match &effect.deps {
            Some(deps) => {
                let parts = deps
                    .iter()
                    .map(|d| self.expr(d))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ");
                format!("[{parts}]")
            }
            None => "undefined".to_string(),
        };
        self.line(&format!("__effect({closure}, {deps});"));
        Ok(())
    }

    /// `@Environment let router: Router` -> `const router = __env("Router");`.
    fn environment_stmt(&mut self, env: &crate::ast::EnvStmt) -> Result<(), XuloError> {
        self.mark_reactive();
        let ty = env.type_.name();
        self.line(&format!("const {} = __env({});", env.name, js_string(&ty)));
        Ok(())
    }

    /// A UI component statement (`VStack { ... }`) compiles to a props-object
    /// call with a `children` array (see README for the calling convention).
    fn component_stmt(&mut self, component: &ComponentStmt) -> Result<(), XuloError> {
        let expr = self.component_props_expr(component)?;
        self.line(&format!("{expr};"));
        Ok(())
    }

    fn component_props_expr(&mut self, component: &ComponentStmt) -> Result<String, XuloError> {
        self.mark_reactive();
        let mut props = Vec::new();
        for (i, arg) in component.args.iter().enumerate() {
            let value = self.expr(&arg.value)?;
            match &arg.name {
                Some(name) => props.push(format!("{}: {value}", js_string(name))),
                None => props.push(format!("\"{i}\": {value}")),
            }
        }
        let children = self.ui_children_expr(&component.children)?;
        props.push(format!("children: {children}"));
        Ok(format!("{}({{ {} }})", component.name, props.join(", ")))
    }

    /// Render a list of UI elements into a `[ ... ]` array expression; `if`,
    /// `for`, and grouped blocks are spread with `...`.
    fn ui_children_expr(&mut self, children: &[UiElement]) -> Result<String, XuloError> {
        let mut parts = Vec::new();
        for child in children {
            match child {
                UiElement::Component(c) => parts.push(self.component_props_expr(c)?),
                UiElement::Text(s) => parts.push(js_string(s)),
                UiElement::Group(group) => {
                    parts.push(format!("...{}", self.ui_children_expr(group)?));
                }
                UiElement::If {
                    condition,
                    then_branch,
                    else_branch,
                } => {
                    let cond = self.expr(condition)?;
                    let then_js = self.ui_children_expr(then_branch)?;
                    let else_js = match else_branch {
                        Some(els) => self.ui_children_expr(els)?,
                        None => "[]".to_string(),
                    };
                    parts.push(format!(
                        "...(() => {{ if ({cond}) {{ return {then_js}; }} else {{ return {else_js}; }} }})()"
                    ));
                }
                UiElement::For {
                    iter_var,
                    iterable,
                    body,
                } => {
                    let iter = self.expr(iterable)?;
                    let body_js = self.ui_children_expr(body)?;
                    parts.push(format!(
                        "...({iter}).map(({iter_var}) => {body_js}).flat()"
                    ));
                }
            }
        }
        Ok(format!("[{}]", parts.join(", ")))
    }

    fn enum_def(&mut self, e: &EnumDef) -> Result<(), XuloError> {
        let has_payload = e.variants.iter().any(|v| v.payload.is_some());
        if has_payload {
            let members = e
                .variants
                .iter()
                .map(|v| {
                    if v.payload.is_some() {
                        format!("{}: (value) => ({{ tag: \"{}\", value: value }})", v.name, v.name)
                    } else {
                        format!("{}: Object.freeze({{ tag: \"{}\" }})", v.name, v.name)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            self.line(&format!("const {} = {{ {members} }};", e.name));
        } else {
            let members = e
                .variants
                .iter()
                .map(|v| format!("{}: \"{}.{}\"", v.name, e.name, v.name))
                .collect::<Vec<_>>()
                .join(", ");
            self.line(&format!("const {} = Object.freeze({{ {members} }});", e.name));
        }
        Ok(())
    }

    fn for_stmt(&mut self, f: &ForStmt) -> Result<(), XuloError> {
        if let Expression::Range(r) = &f.iterable {
            let start = self.expr(&r.start)?;
            let end = self.expr(&r.end)?;
            self.line(&format!(
                "for (let {} = {start}; {} < {end}; {}++) {{",
                f.iter_var, f.iter_var, f.iter_var
            ));
            self.indent += 1;
            self.block_body(&f.body)?;
            self.indent -= 1;
            self.line("}");
            return Ok(());
        }
        let iterable = self.expr(&f.iterable)?;
        self.line(&format!("for (const {} of {iterable}) {{", f.iter_var));
        self.indent += 1;
        self.block_body(&f.body)?;
        self.indent -= 1;
        self.line("}");
        Ok(())
    }

    /// A `fn ...(): Component` compiles to a function whose `@State`/`@Store`/
    /// `@Effect`/`@Environment` declarations are hoisted into setup code, and
    /// whose remaining body (the UI) runs inside `__component(function(){...})`
    /// so signal changes trigger a re-render.
    fn component_fn_def(&mut self, f: &FnDef) -> Result<(), XuloError> {
        self.mark_reactive();
        let params = f
            .params
            .iter()
            .map(|p| {
                let base = p.name.clone();
                match &p.default {
                    Some(d) => {
                        let d = self.expr(d)?;
                        Ok::<_, XuloError>(format!("{base} = {d}"))
                    }
                    None => Ok(base),
                }
            })
            .collect::<Result<Vec<_>, XuloError>>()?
            .join(", ");
        self.line(&format!("function {}({params}) {{", f.name));
        self.indent += 1;
        self.push_signal_scope();
        for statement in &f.body.statements {
            match statement {
                Statement::State(state) => self.state_stmt(&state.binding)?,
                Statement::Store(store) => self.store_stmt(store)?,
                Statement::Effect(effect) => self.effect_stmt(effect)?,
                Statement::Environment(env) => self.environment_stmt(env)?,
                _ => {}
            }
        }
        let rest: Vec<&Statement> = f
            .body
            .statements
            .iter()
            .filter(|s| {
                !matches!(
                    s,
                    Statement::State(_)
                        | Statement::Store(_)
                        | Statement::Effect(_)
                        | Statement::Environment(_)
                )
            })
            .collect();
        self.line("return __component(function() {");
        self.indent += 1;
        if let Some((last, prefix)) = rest.split_last() {
            for statement in prefix {
                self.statement(statement)?;
            }
            match last {
                Statement::Expr(expr) => {
                    let value = self.expr(expr)?;
                    self.line(&format!("return {value};"));
                }
                Statement::Component(component) => {
                    let value = self.component_props_expr(component)?;
                    self.line(&format!("return {value};"));
                }
                other => self.statement(other)?,
            }
        }
        self.indent -= 1;
        self.line("});");
        self.pop_signal_scope();
        self.indent -= 1;
        self.line("}");
        Ok(())
    }

    fn while_stmt(&mut self, w: &WhileStmt) -> Result<(), XuloError> {
        let condition = self.expr(&w.condition)?;
        self.line(&format!("while ({condition}) {{"));
        self.indent += 1;
        self.block_body(&w.body)?;
        self.indent -= 1;
        self.line("}");
        Ok(())
    }

    fn try_stmt(&mut self, t: &crate::ast::TryStmt) -> Result<(), XuloError> {
        self.line("try {");
        self.indent += 1;
        self.block_body(&t.try_block)?;
        self.indent -= 1;
        self.line(&format!("}} catch ({}) {{", t.catch_var));
        self.indent += 1;
        self.block_body(&t.catch_block)?;
        self.indent -= 1;
        self.line("}");
        Ok(())
    }

    fn if_stmt(&mut self, if_expr: &IfExpr) -> Result<(), XuloError> {
        let condition = self.expr(&if_expr.condition)?;
        self.line(&format!("if ({condition}) {{"));
        self.indent += 1;
        self.block_body(&if_expr.then_branch)?;
        self.indent -= 1;
        match &if_expr.else_branch {
            Some(else_block) if is_else_if(else_block) => {
                // else if (c) { ... }
                let inner = else_block.statements.first().unwrap();
                if let Statement::Expr(Expression::If(nested)) = inner {
                    let condition = self.expr(&nested.condition)?;
                    self.line(&format!("}} else if ({condition}) {{"));
                    self.indent += 1;
                    self.block_body(&nested.then_branch)?;
                    self.indent -= 1;
                    self.emit_tail_else(&nested.else_branch)?;
                }
            }
            Some(else_block) => {
                self.line("} else {");
                self.indent += 1;
                self.block_body(else_block)?;
                self.indent -= 1;
                self.line("}");
            }
            None => self.line("}"),
        }
        Ok(())
    }

    fn emit_tail_else(&mut self, else_branch: &Option<Block>) -> Result<(), XuloError> {
        match else_branch {
            Some(b) if is_else_if(b) => {
                if let Statement::Expr(Expression::If(nested)) = b.statements.first().unwrap() {
                    let condition = self.expr(&nested.condition)?;
                    self.line(&format!("}} else if ({condition}) {{"));
                    self.indent += 1;
                    self.block_body(&nested.then_branch)?;
                    self.indent -= 1;
                    self.emit_tail_else(&nested.else_branch)?;
                }
            }
            Some(b) => {
                self.line("} else {");
                self.indent += 1;
                self.block_body(b)?;
                self.indent -= 1;
                self.line("}");
            }
            None => self.line("}"),
        }
        Ok(())
    }

    fn block_body(&mut self, block: &Block) -> Result<(), XuloError> {
        for statement in &block.statements {
            self.statement(statement)?;
        }
        Ok(())
    }

    /// Render an expression.
    fn expr(&mut self, expr: &Expression) -> Result<String, XuloError> {
        Ok(match expr {
            Expression::Literal(lit) => self.literal(lit)?,
            Expression::Identifier(name) => {
                if self.is_signal(name) {
                    format!("{name}.get()")
                } else {
                    name.clone()
                }
            }
            Expression::BinaryOp(bin) => self.binary_op(bin)?,
            Expression::Unary(un) => format!("({}{})", un.operator.symbol(), self.expr(&un.operand)?),
            Expression::Call(call) => self.call(call)?,
            Expression::EnumRef(r) => format!("{}.{}", r.enum_name, r.variant),
            Expression::If(if_expr) => self.expr_if(if_expr)?,
            Expression::Ternary(tr) => format!(
                "({} ? {} : {})",
                self.expr(&tr.condition)?,
                self.expr(&tr.then_value)?,
                self.expr(&tr.else_value)?
            ),
            Expression::Match(m) => self.expr_match(m)?,
            Expression::Member(m) => {
                let dot = if m.optional { "?." } else { "." };
                format!("{}{}{}", self.expr(&m.object)?, dot, m.property)
            }
            Expression::Index(idx) => {
                format!("{}[{}]", self.expr(&idx.object)?, self.expr(&idx.index)?)
            }
            Expression::Nullish(n) => format!(
                "({} ?? {})",
                self.expr(&n.left)?,
                self.expr(&n.right)?
            ),
            Expression::Range(r) => {
                let start = self.expr(&r.start)?;
                let end = self.expr(&r.end)?;
                format!("range({start}, {end})")
            }
            Expression::Await(operand) => format!("(await {})", self.expr(operand)?),
            Expression::FnExpr(f) => self.fn_expr(f)?,
            Expression::Binding(name) => {
                if self.is_signal(name) {
                    format!(
                        "{{ value: {name}.get(), onChange: (__v) => {name}.set(__v) }}"
                    )
                } else {
                    format!("{{ value: {name}, onChange: (__v) => {{}} }}")
                }
            }
            Expression::Spread(_) => unreachable!("spread handled inside list/object literals"),
            Expression::CallValue(cv) => self.call_value(cv)?,
        })
    }

    /// Call a function value held in an arbitrary expression: `(xs[0])(10)`.
    fn call_value(&mut self, cv: &CallValue) -> Result<String, XuloError> {
        let callee = self.expr(&cv.callee)?;
        let args = cv
            .arguments
            .iter()
            .map(|a| self.expr(&a.value))
            .collect::<Result<Vec<_>, XuloError>>()?;
        Ok(format!("({callee})({})", args.join(", ")))
    }

    /// `fn(a, b) { ... }` closes over the enclosing scope; a declared return
    /// type makes a trailing expression statement the implicit return.
    fn fn_expr(&mut self, f: &crate::ast::FnExpr) -> Result<String, XuloError> {
        let params = f
            .params
            .iter()
            .map(|p| {
                let base = p.name.clone();
                match &p.default {
                    Some(d) => Ok::<_, XuloError>(format!("{base} = {}", self.expr(d)?)),
                    None => Ok(base),
                }
            })
            .collect::<Result<Vec<_>, _>>()?
            .join(", ");
        let kw = if f.is_async { "async function" } else { "function" };
        let mut body = String::new();
        let stmts = &f.body.statements;
        if f.return_type.is_some()
            && let Some(Statement::Expr(last)) = stmts.last()
        {
            let mut inline = self.child();
            inline.indent = 1;
            for s in &stmts[..stmts.len() - 1] {
                inline.statement(s)?;
            }
            let value = inline.expr(last)?;
            body.push_str(&format!("    return {value};\n"));
        } else {
            for s in stmts {
                let mut inline = self.child();
                inline.indent = 1;
                inline.statement(s)?;
                body.push_str(&inline.finish());
            }
        }
        Ok(format!("({kw} ({params}) {{\n{body}}})"))
    }

    fn literal(&mut self, lit: &Literal) -> Result<String, XuloError> {
        Ok(match lit {
            Literal::String(s) => js_string(s),
            Literal::Number(n) => fmt_number(*n),
            Literal::Boolean(b) => b.to_string(),
            Literal::Null => "null".to_string(),
            Literal::List(items) => {
                let elems = items
                    .iter()
                    .map(|e| match e {
                        Expression::Spread(spread) => {
                            Ok(format!("...{}", self.expr(spread)?))
                        }
                        other => self.expr(other),
                    })
                    .collect::<Result<Vec<_>, XuloError>>()?;
                format!("[{}]", elems.join(", "))
            }
            Literal::Object(fields) => {
                let parts = fields
                    .iter()
                    .map(|f| match f {
                        ObjectField::Field { name, value } => {
                            Ok(format!("{}: {}", js_string(name), self.expr(value)?))
                        }
                        ObjectField::Spread { value } => {
                            Ok(format!("...{}", self.expr(value)?))
                        }
                    })
                    .collect::<Result<Vec<_>, XuloError>>()?;
                format!("{{{}}}", parts.join(", "))
            }
        })
    }

    fn binary_op(&mut self, bin: &BinaryOp) -> Result<String, XuloError> {
        let left = self.expr(&bin.left)?;
        let right = self.expr(&bin.right)?;
        let symbol = match bin.operator {
            BinaryOperator::And => "&&",
            BinaryOperator::Or => "||",
            other => other.symbol(),
        };
        Ok(format!("({left} {symbol} {right})"))
    }

    fn call(&mut self, call: &Call) -> Result<String, XuloError> {
        if let Some((enum_name, variant)) = call.enum_parts() {
            let args = self.call_args_ordered(call, None)?;
            Ok(format!("{enum_name}.{variant}({args})"))
        } else if let Some(object) = &call.object {
            let receiver = self.expr(object)?;
            let method = call.method.as_deref().unwrap_or("");
            let args = self.call_args_ordered(call, None)?;
            Ok(format!("{receiver}.{method}({args})"))
        } else if call.callee == "print" {
            let joined = call
                .arguments
                .iter()
                .map(|a| self.expr(&a.value))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ");
            Ok(format!("console.log({joined})"))
        } else {
            let params = self.fn_params.get(&call.callee).cloned();
            let args = self.call_args_ordered(call, params.as_ref())?;
            Ok(format!("{}({args})", call.callee))
        }
    }

    /// Emit call arguments. Named arguments are reordered to match the
    /// callee's declared parameter order (defaults may be omitted).
    fn call_args_ordered(
        &mut self,
        call: &Call,
        param_names: Option<&Vec<String>>,
    ) -> Result<String, XuloError> {
        let all_named = !call.arguments.is_empty()
            && call.arguments.iter().all(|a| a.name.is_some());
        if !all_named {
            return call
                .arguments
                .iter()
                .map(|a| self.expr(&a.value))
                .collect::<Result<Vec<_>, _>>()
                .map(|v| v.join(", "));
        }
        let Some(params) = param_names else {
            return Err(XuloError::new(
                crate::error::ErrorKind::Codegen,
                format!("named arguments require parameter names for `{}`", call.callee),
            ));
        };
        let mut by_name = std::collections::HashMap::new();
        for a in &call.arguments {
            if let Some(name) = &a.name {
                by_name.insert(name.clone(), self.expr(&a.value)?);
            }
        }
        let ordered = params
            .iter()
            .filter_map(|name| by_name.get(name).cloned())
            .collect::<Vec<_>>()
            .join(", ");
        Ok(ordered)
    }

    /// `if` in a value position is emitted as an IIFE whose arms render their
/// blocks inline (so `return`, assignments, etc. inside the arms work).
    fn expr_if(&mut self, if_expr: &IfExpr) -> Result<String, XuloError> {
        let condition = self.expr(&if_expr.condition)?;
        let then = self.block_inline(&if_expr.then_branch)?;
        let els = match &if_expr.else_branch {
            Some(b) => self.block_inline(b)?,
            None => "return undefined;".to_string(),
        };
        Ok(format!(
            "(() => {{ if ({condition}) {{\n{then}\n}} else {{\n{els}\n}} }})()"
        ))
    }

    /// Render a block as the inline statements of an IIFE arm. A trailing
    /// expression statement (or `return`) becomes the arm's `return`.
    fn block_inline(&mut self, block: &Block) -> Result<String, XuloError> {
        let mut js = self.child();
        js.indent = self.indent + 1;
        match block.statements.last() {
            Some(Statement::Expr(e)) => {
                for s in &block.statements[..block.statements.len() - 1] {
                    js.statement(s)?;
                }
                let value = js.expr(e)?;
                js.line(&format!("return {value};"));
            }
            Some(Statement::Return(r)) => {
                for s in &block.statements[..block.statements.len() - 1] {
                    js.statement(s)?;
                }
                let value = js.expr(&r.value)?;
                js.line(&format!("return {value};"));
            }
            _ => js.block_body(block)?,
        }
        Ok(js.finish())
    }

    /// `match` in a value position compiles to an IIFE that compares the
    /// scrutinee against each arm and returns the first match.
    fn expr_match(&mut self, m: &crate::ast::MatchExpr) -> Result<String, XuloError> {
        let mut js = self.child();
        js.indent = self.indent + 1;
        let scrutinee = self.expr(&m.value)?;
        js.line(&format!("const __m = {scrutinee};"));
        for arm in &m.arms {
            let value = self.expr(&arm.value)?;
            match &arm.pattern {
                crate::ast::MatchPattern::Wildcard => {
                    js.line(&format!("return {value};"));
                }
                crate::ast::MatchPattern::Literal(lit) => {
                    let ljs = self.literal(lit)?;
                    js.line(&format!("if (__m === {ljs}) {{"));
                    js.indent += 1;
                    js.line(&format!("return {value};"));
                    js.indent -= 1;
                    js.line("}");
                }
                crate::ast::MatchPattern::Enum(r) => {
                    // Payload-capable enums use `{tag}` objects; payload-less
                    // enums use `"Enum.Variant"` strings. Accept either
                    // representation so one code path matches both.
                    js.line(&format!(
                        "if (__m === \"{}.{}\" || (__m && __m.tag === \"{}\")) {{",
                        r.enum_name, r.variant, r.variant
                    ));
                    js.indent += 1;
                    js.line(&format!("return {value};"));
                    js.indent -= 1;
                    js.line("}");
                }
                crate::ast::MatchPattern::EnumPayload {
                    enum_name: _,
                    variant,
                    binding,
                } => {
                    js.line(&format!("if (__m && __m.tag === \"{variant}\") {{"));
                    js.indent += 1;
                    js.line(&format!("const {binding} = __m.value;"));
                    js.line(&format!("return {value};"));
                    js.indent -= 1;
                    js.line("}");
                }
            }
        }
        let has_wildcard = m
            .arms
            .iter()
            .any(|a| matches!(a.pattern, crate::ast::MatchPattern::Wildcard));
        if !has_wildcard {
            js.line("throw new Error(\"non-exhaustive match\");");
        }
        Ok(format!("(() => {{\n{} }})()", js.finish()))
    }
}

/// A block consisting of exactly one `if` statement represents `else if`.
fn is_else_if(block: &Block) -> bool {
    matches!(block.statements.as_slice(), [Statement::Expr(Expression::If(_))])
}

/// True when the program's `main` (plain, `export fn`, or `export default fn`)
/// returns `Component`.
pub fn main_returns_component(program: &Program) -> bool {
    let main_fn = program.statements.iter().find_map(|s| match s {
        Statement::Fn(f) if f.name == "main" => Some(f),
        Statement::Export(export) => match &export.item {
            crate::ast::ExportItem::Fn(f) if f.name == "main" => Some(f),
            crate::ast::ExportItem::Default(inner) => match inner.as_ref() {
                crate::ast::ExportItem::Fn(f) if f.name == "main" => Some(f),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    });
    matches!(
        main_fn.and_then(|f| f.return_type.as_ref()),
        Some(Type::Named(n)) if n == "Component"
    )
}

fn fmt_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// JSON-style string escaping for JavaScript.
fn js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}