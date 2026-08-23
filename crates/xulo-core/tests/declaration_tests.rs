use xulo_core::ast::*;

#[test]
fn test_fn_def_basic() {
    let fn_def = FnDef {
        name: "main".to_string(),
        name_span: 3..7,
        params: vec![],
        return_type: None,
        type_params: vec![],
        bounds: vec![],
        is_async: false,
        body: Block {
            statements: vec![],
        },
        span: 0..10,
    };
    assert_eq!(fn_def.name, "main");
    assert!(fn_def.params.is_empty());
    assert!(fn_def.return_type.is_none());
    assert!(!fn_def.is_async);
}

#[test]
fn test_fn_def_with_params() {
    let fn_def = FnDef {
        name: "add".to_string(),
        name_span: 3..6,
        params: vec![
            Param {
                name: "a".to_string(),
                type_annotation: Some(Type::Number),
                default: None,
                span: 7..8,
            },
            Param {
                name: "b".to_string(),
                type_annotation: Some(Type::Number),
                default: None,
                span: 10..11,
            },
        ],
        return_type: Some(Type::Number),
        type_params: vec![],
        bounds: vec![],
        is_async: false,
        body: Block {
            statements: vec![],
        },
        span: 0..15,
    };
    assert_eq!(fn_def.params.len(), 2);
    assert!(fn_def.return_type.is_some());
}

#[test]
fn test_fn_def_async() {
    let fn_def = FnDef {
        name: "fetch".to_string(),
        name_span: 9..14,
        params: vec![],
        return_type: Some(Type::Async(Box::new(Type::String))),
        type_params: vec![],
        bounds: vec![],
        is_async: true,
        body: Block {
            statements: vec![],
        },
        span: 0..20,
    };
    assert!(fn_def.is_async);
}

#[test]
fn test_fn_def_generic() {
    let fn_def = FnDef {
        name: "identity".to_string(),
        name_span: 3..11,
        params: vec![Param {
            name: "x".to_string(),
            type_annotation: Some(Type::Named("T".to_string())),
            default: None,
            span: 12..13,
        }],
        return_type: Some(Type::Named("T".to_string())),
        type_params: vec!["T".to_string()],
        bounds: vec![FnBound {
            param: "T".to_string(),
            traits: vec!["Clone".to_string()],
        }],
        is_async: false,
        body: Block {
            statements: vec![],
        },
        span: 0..20,
    };
    assert_eq!(fn_def.type_params, vec!["T".to_string()]);
    assert_eq!(fn_def.bounds.len(), 1);
    assert_eq!(fn_def.bounds[0].traits, vec!["Clone".to_string()]);
}

#[test]
fn test_let_binding_basic() {
    let binding = LetBinding {
        name: "x".to_string(),
        name_span: 4..5,
        type_annotation: None,
        value: Some(Expression::Literal {
            value: Literal::Number(42.0),
            span: 8..10,
        }),
        is_const: false,
        is_mutable: false,
        memo: false,
        memo_deps: None,
        tuple_names: None,
        object_destructuring: None,
    };
    assert_eq!(binding.name, "x");
    assert!(!binding.is_const);
    assert!(!binding.memo);
}

#[test]
fn test_let_binding_const() {
    let binding = LetBinding {
        name: "MAX".to_string(),
        name_span: 6..9,
        type_annotation: Some(Type::Number),
        value: Some(Expression::Literal {
            value: Literal::Number(100.0),
            span: 12..15,
        }),
        is_const: true,
        is_mutable: false,
        memo: false,
        memo_deps: None,
        tuple_names: None,
        object_destructuring: None,
    };
    assert!(binding.is_const);
}

#[test]
fn test_let_binding_memo() {
    let binding = LetBinding {
        name: "cached".to_string(),
        name_span: 6..12,
        type_annotation: None,
        value: Some(Expression::Identifier {
            name: "expensive".to_string(),
            span: 15..24,
        }),
        is_const: false,
        is_mutable: false,
        memo: true,
        memo_deps: Some(vec![
            Expression::Identifier {
                name: "dep1".to_string(),
                span: 31..35,
            },
            Expression::Identifier {
                name: "dep2".to_string(),
                span: 37..41,
            },
        ]),
        tuple_names: None,
        object_destructuring: None,
    };
    assert!(binding.memo);
    assert_eq!(binding.memo_deps.as_ref().unwrap().len(), 2);
}

#[test]
fn test_return_stmt_with_value() {
    let ret = ReturnStmt {
        value: Some(Expression::Literal {
            value: Literal::Number(42.0),
            span: 7..9,
        }),
        span: 0..9,
    };
    assert!(ret.value.is_some());
}

#[test]
fn test_return_stmt_without_value() {
    let ret = ReturnStmt {
        value: None,
        span: 0..7,
    };
    assert!(ret.value.is_none());
}
