use super::*;

pub(super) fn block_has_closure(block: &Block) -> bool {
    block.statements.iter().any(stmt_has_closure)
}

pub(super) fn stmt_has_closure(stmt: &Statement) -> bool {
    match stmt {
        Statement::Fn(_) => true,
        // A `#Test fn` body is inert: nothing can reach it, so its closures
        // never exist at run time.
        Statement::Test(_) => false,
        Statement::Let(l) => {
            l.value.as_ref().is_some_and(expr_has_closure)
                || l.memo_deps.iter().flatten().any(expr_has_closure)
        }
        Statement::Return(r) => r.value.as_ref().is_some_and(expr_has_closure),
        Statement::For(f) => expr_has_closure(&f.iterable) || block_has_closure(&f.body),
        Statement::While(w) => expr_has_closure(&w.condition) || block_has_closure(&w.body),
        Statement::Assign(a) => {
            let target = match &a.target {
                AssignTarget::Name(_) => false,
                AssignTarget::Member(object, _) => expr_has_closure(object),
                AssignTarget::Index(object, index) => {
                    expr_has_closure(object) || expr_has_closure(index)
                }
            };
            target || expr_has_closure(&a.value)
        }
        Statement::TypeAlias(_)
        | Statement::Enum(_)
        | Statement::Struct(_)
        | Statement::Mod(_)
        | Statement::Use(_)
        | Statement::Environment(_)
        | Statement::Trait(_)
        | Statement::Break
        | Statement::Continue => false,
        Statement::Expr(e) => expr_has_closure(&e.expr),
        Statement::Block(b) => block_has_closure(b),
        Statement::Defer(d) => match &d.body {
            xulo_core::ast::DeferBody::Block(b) => block_has_closure(b),
            xulo_core::ast::DeferBody::Expr(e) => expr_has_closure(e),
        },
        Statement::Try(t) => block_has_closure(&t.try_block) || block_has_closure(&t.catch_block),
        Statement::Throw(e) => expr_has_closure(e),
        Statement::Export(e) => match &e.item {
            xulo_core::ast::ExportItem::Fn(_) => true,
            xulo_core::ast::ExportItem::Let(l) => l.value.as_ref().is_some_and(expr_has_closure),
            xulo_core::ast::ExportItem::Enum(_)
            | xulo_core::ast::ExportItem::Type(_)
            | xulo_core::ast::ExportItem::Struct(_)
            | xulo_core::ast::ExportItem::Trait(_)
            | xulo_core::ast::ExportItem::Names(_) => false,
        },
        Statement::State(s) => s.binding.value.as_ref().is_some_and(expr_has_closure),
        Statement::Store(s) => expr_has_closure(&s.value),
        Statement::Effect(_) => true,
        Statement::Component(c) => {
            c.args.iter().any(|a| expr_has_closure(&a.value))
                || c.children.iter().any(ui_has_closure)
        }
        Statement::Impl(_) => true,
    }
}

pub(super) fn expr_has_closure(expr: &Expression) -> bool {
    match expr {
        Expression::Literal { value, .. } => match value {
            Literal::List(items) => items.iter().any(expr_has_closure),
            Literal::Object(fields) => fields.iter().any(|field| match field {
                ObjectField::Field { value, .. } => expr_has_closure(value),
                ObjectField::Spread { value } => expr_has_closure(value),
            }),
            Literal::String(_) | Literal::Number(_) | Literal::Boolean(_) | Literal::Null => false,
        },
        Expression::Identifier { .. } | Expression::EnumRef(_) | Expression::Binding { .. } => {
            false
        }
        Expression::BinaryOp(b) => expr_has_closure(&b.left) || expr_has_closure(&b.right),
        Expression::Unary(u) => expr_has_closure(&u.operand),
        Expression::Call(c) => {
            c.object.as_deref().is_some_and(expr_has_closure)
                || c.arguments.iter().any(|a| expr_has_closure(&a.value))
        }
        Expression::If(e) => {
            expr_has_closure(&e.condition)
                || block_has_closure(&e.then_branch)
                || e.else_branch.as_ref().is_some_and(block_has_closure)
        }
        Expression::Ternary(t) => {
            expr_has_closure(&t.condition)
                || expr_has_closure(&t.then_value)
                || expr_has_closure(&t.else_value)
        }
        Expression::Match(m) => {
            expr_has_closure(&m.value)
                || m.arms.iter().any(|arm| {
                    arm.guard.as_ref().is_some_and(expr_has_closure)
                        || match_body_has_closure(&arm.value)
                })
        }
        Expression::Member(m) => expr_has_closure(&m.object),
        Expression::Index(i) => expr_has_closure(&i.object) || expr_has_closure(&i.index),
        Expression::Nullish(n) => expr_has_closure(&n.left) || expr_has_closure(&n.right),
        Expression::Range(r) => expr_has_closure(&r.start) || expr_has_closure(&r.end),
        Expression::Await { expr, .. } => expr_has_closure(expr),
        Expression::FnExpr(_) => true,
        Expression::Spread { expr, .. } => expr_has_closure(expr),
        Expression::CallValue(c) => {
            expr_has_closure(&c.callee) || c.arguments.iter().any(|a| expr_has_closure(&a.value))
        }
        Expression::Component(c) => {
            c.args.iter().any(|a| expr_has_closure(&a.value))
                || c.children.iter().any(ui_has_closure)
        }
        Expression::Tuple { items, .. } => items.iter().any(expr_has_closure),
        Expression::TypedMap(tm) => tm
            .entries
            .iter()
            .any(|e| expr_has_closure(&e.key) || expr_has_closure(&e.value)),
        Expression::Is(is) => expr_has_closure(&is.expr),
        Expression::Panic { expr, .. } => expr_has_closure(expr),
        Expression::Propagate { expr, .. } => expr_has_closure(expr),
        Expression::Arrow(_) => true,
        Expression::Spawn(s) => match &s.body {
            xulo_core::ast::SpawnBody::Async(b) => block_has_closure(b),
            xulo_core::ast::SpawnBody::Service { fields, .. } => {
                fields.iter().any(|f| expr_has_closure(&f.value))
            }
        },
        Expression::Lock(l) => block_has_closure(&l.body),
    }
}

pub(super) fn match_body_has_closure(body: &xulo_core::ast::MatchBody) -> bool {
    match body {
        xulo_core::ast::MatchBody::Expr(e) => expr_has_closure(e),
        xulo_core::ast::MatchBody::Block(b) => block_has_closure(b),
    }
}

pub(super) fn ui_has_closure(el: &xulo_core::ast::UiElement) -> bool {
    match el {
        xulo_core::ast::UiElement::Component(c) => {
            c.args.iter().any(|a| expr_has_closure(&a.value))
                || c.children.iter().any(ui_has_closure)
        }
        xulo_core::ast::UiElement::Text(_) => false,
        xulo_core::ast::UiElement::Expr(e) => expr_has_closure(e),
        xulo_core::ast::UiElement::If {
            condition,
            then_branch,
            else_branch,
        } => {
            expr_has_closure(condition)
                || then_branch.iter().any(ui_has_closure)
                || else_branch
                    .as_ref()
                    .is_some_and(|branch| branch.iter().any(ui_has_closure))
        }
        xulo_core::ast::UiElement::For { iterable, body, .. } => {
            expr_has_closure(iterable) || body.iter().any(ui_has_closure)
        }
        xulo_core::ast::UiElement::Group(children) => children.iter().any(ui_has_closure),
    }
}
