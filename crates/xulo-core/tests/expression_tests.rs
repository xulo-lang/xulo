use xulo_core::ast::*;

#[test]
fn test_expression_span_literal() {
    let expr = Expression::Literal {
        value: Literal::Number(42.0),
        span: 5..7,
    };
    assert_eq!(expr.span(), &(5..7));
}

#[test]
fn test_expression_span_identifier() {
    let expr = Expression::Identifier {
        name: "x".to_string(),
        span: 10..11,
    };
    assert_eq!(expr.span(), &(10..11));
}

#[test]
fn test_expression_span_binary_op() {
    let expr = Expression::BinaryOp(Box::new(BinaryOp {
        left: Expression::Identifier {
            name: "a".to_string(),
            span: 0..1,
        },
        operator: BinaryOperator::Add,
        right: Expression::Identifier {
            name: "b".to_string(),
            span: 4..5,
        },
        span: 0..5,
        list_concat: false,
    }));
    assert_eq!(expr.span(), &(0..5));
}

#[test]
fn test_expression_span_call() {
    let expr = Expression::Call(Call {
        callee: "foo".to_string(),
        callee_span: Some(0..3),
        object: None,
        method: None,
        optional: false,
        arguments: vec![],
        span: 0..5,
        trait_impl: None,
    });
    assert_eq!(expr.span(), &(0..5));
}

#[test]
fn test_expression_span_unary() {
    let expr = Expression::Unary(Box::new(UnaryOp {
        operator: UnaryOperator::Neg,
        operand: Expression::Identifier {
            name: "x".to_string(),
            span: 1..2,
        },
        span: 0..2,
    }));
    assert_eq!(expr.span(), &(0..2));
}

#[test]
fn test_expression_span_await() {
    let expr = Expression::Await {
        expr: Box::new(Expression::Identifier {
            name: "p".to_string(),
            span: 5..6,
        }),
        span: 1..6,
    };
    assert_eq!(expr.span(), &(1..6));
}

#[test]
fn test_expression_span_spread() {
    let expr = Expression::Spread {
        expr: Box::new(Expression::Identifier {
            name: "xs".to_string(),
            span: 3..5,
        }),
        span: 0..5,
    };
    assert_eq!(expr.span(), &(0..5));
}

#[test]
fn test_expression_span_binding() {
    let expr = Expression::Binding {
        name: "count".to_string(),
        span: 0..5,
    };
    assert_eq!(expr.span(), &(0..5));
}

#[test]
fn test_expression_span_member() {
    let expr = Expression::Member(Box::new(MemberAccess {
        object: Expression::Identifier {
            name: "obj".to_string(),
            span: 0..3,
        },
        property: "x".to_string(),
        optional: false,
        span: 0..5,
    }));
    assert_eq!(expr.span(), &(0..5));
}

#[test]
fn test_expression_span_index() {
    let expr = Expression::Index(Box::new(IndexExpr {
        object: Box::new(Expression::Identifier {
            name: "xs".to_string(),
            span: 0..2,
        }),
        index: Box::new(Expression::Literal {
            value: Literal::Number(0.0),
            span: 3..4,
        }),
        span: 0..5,
    }));
    assert_eq!(expr.span(), &(0..5));
}
