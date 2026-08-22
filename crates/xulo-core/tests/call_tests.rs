use xulo_core::ast::*;

#[test]
fn test_call_is_enum() {
    let call = Call {
        callee: "Result::Success".to_string(),
        callee_span: None,
        object: None,
        method: None,
        optional: false,
        arguments: vec![],
        span: 0..15,
        trait_impl: None,
    };
    assert!(call.is_enum());
}

#[test]
fn test_call_is_not_enum_with_object() {
    let call = Call {
        callee: "Result::Success".to_string(),
        callee_span: None,
        object: Some(Box::new(Expression::Identifier {
            name: "x".to_string(),
            span: 0..1,
        })),
        method: Some("Success".to_string()),
        optional: false,
        arguments: vec![],
        span: 0..15,
        trait_impl: None,
    };
    assert!(!call.is_enum());
}

#[test]
fn test_call_is_not_enum_plain() {
    let call = Call {
        callee: "foo".to_string(),
        callee_span: Some(0..3),
        object: None,
        method: None,
        optional: false,
        arguments: vec![],
        span: 0..5,
        trait_impl: None,
    };
    assert!(!call.is_enum());
}

#[test]
fn test_call_enum_parts() {
    let call = Call {
        callee: "Result::Success".to_string(),
        callee_span: None,
        object: None,
        method: None,
        optional: false,
        arguments: vec![],
        span: 0..15,
        trait_impl: None,
    };
    assert_eq!(call.enum_parts(), Some(("Result", "Success")));
}

#[test]
fn test_call_enum_parts_with_object() {
    let call = Call {
        callee: "Result::Success".to_string(),
        callee_span: None,
        object: Some(Box::new(Expression::Identifier {
            name: "x".to_string(),
            span: 0..1,
        })),
        method: Some("Success".to_string()),
        optional: false,
        arguments: vec![],
        span: 0..15,
        trait_impl: None,
    };
    assert_eq!(call.enum_parts(), None);
}

#[test]
fn test_call_optional() {
    let call = Call {
        callee: "method".to_string(),
        callee_span: Some(2..8),
        object: Some(Box::new(Expression::Identifier {
            name: "x".to_string(),
            span: 0..1,
        })),
        method: Some("method".to_string()),
        optional: true,
        arguments: vec![],
        span: 0..10,
        trait_impl: None,
    };
    assert!(call.optional);
}

#[test]
fn test_call_with_trait_impl() {
    let call = Call {
        callee: "Area::area".to_string(),
        callee_span: None,
        object: None,
        method: None,
        optional: false,
        arguments: vec![CallArg {
            name: None,
            value: Expression::Identifier {
                name: "self".to_string(),
                span: 10..14,
            },
        }],
        span: 0..15,
        trait_impl: Some("impl_Area_Rectangle_area".to_string()),
    };
    assert_eq!(
        call.trait_impl,
        Some("impl_Area_Rectangle_area".to_string())
    );
}

#[test]
fn test_call_arg_labeled() {
    let arg = CallArg {
        name: Some("x".to_string()),
        value: Expression::Literal {
            value: Literal::Number(1.0),
            span: 4..5,
        },
    };
    assert_eq!(arg.name, Some("x".to_string()));
}

#[test]
fn test_call_arg_positional() {
    let arg = CallArg {
        name: None,
        value: Expression::Literal {
            value: Literal::Number(1.0),
            span: 0..1,
        },
    };
    assert!(arg.name.is_none());
}
