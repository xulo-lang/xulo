use xulo_core::ast::*;

#[test]
fn test_literal_string() {
    let lit = Literal::String("hello".to_string());
    assert!(matches!(lit, Literal::String(s) if s == "hello"));
}

#[test]
fn test_literal_number() {
    let lit = Literal::Float(42.5);
    assert!(matches!(lit, Literal::Float(n) if n == 42.5));
    let int = Literal::Int(42);
    assert!(matches!(int, Literal::Int(n) if n == 42));
}

#[test]
fn test_literal_boolean() {
    assert!(matches!(Literal::Boolean(true), Literal::Boolean(true)));
    assert!(matches!(Literal::Boolean(false), Literal::Boolean(false)));
}

#[test]
fn test_literal_null() {
    assert!(matches!(Literal::Null, Literal::Null));
}

#[test]
fn test_literal_list() {
    let lit = Literal::List(vec![
        Expression::Literal {
            value: Literal::Int(1),
            span: 0..1,
        },
        Expression::Literal {
            value: Literal::Int(2),
            span: 2..3,
        },
    ]);
    match lit {
        Literal::List(elems) => assert_eq!(elems.len(), 2),
        _ => panic!("expected list"),
    }
}

#[test]
fn test_literal_object() {
    let lit = Literal::Object(vec![ObjectField::Field {
        name: "x".to_string(),
        value: Expression::Literal {
            value: Literal::Int(1),
            span: 4..5,
        },
    }]);
    match lit {
        Literal::Object(fields) => assert_eq!(fields.len(), 1),
        _ => panic!("expected object"),
    }
}

#[test]
fn test_object_field_spread() {
    let field = ObjectField::Spread {
        value: Expression::Identifier {
            name: "rest".to_string(),
            span: 5..9,
        },
    };
    assert!(matches!(field, ObjectField::Spread { .. }));
}
