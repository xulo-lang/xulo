use xulo_core::ast::*;

#[test]
fn test_enum_def_basic() {
    let enum_def = EnumDef {
        name: "Color".to_string(),
        name_span: 5..10,
        type_params: vec![],
        variants: vec![
            EnumVariant {
                name: "Red".to_string(),
                name_span: 12..15,
                payload: None,
            },
            EnumVariant {
                name: "Green".to_string(),
                name_span: 17..22,
                payload: None,
            },
        ],
    };
    assert_eq!(enum_def.name, "Color");
    assert_eq!(enum_def.variants.len(), 2);
}

#[test]
fn test_enum_def_with_payload() {
    let enum_def = EnumDef {
        name: "Result".to_string(),
        name_span: 5..11,
        type_params: vec![],
        variants: vec![
            EnumVariant {
                name: "Success".to_string(),
                name_span: 13..20,
                payload: Some(vec![EnumPayloadParam {
                    name: Some("data".to_string()),
                    type_: Type::Any,
                }]),
            },
            EnumVariant {
                name: "Error".to_string(),
                name_span: 22..27,
                payload: Some(vec![EnumPayloadParam {
                    name: None,
                    type_: Type::String,
                }]),
            },
        ],
    };
    assert_eq!(enum_def.variants[0].payload.as_ref().unwrap().len(), 1);
    assert_eq!(enum_def.variants[1].payload.as_ref().unwrap().len(), 1);
}

#[test]
fn test_enum_def_generic() {
    let enum_def = EnumDef {
        name: "Option".to_string(),
        name_span: 5..11,
        type_params: vec!["T".to_string()],
        variants: vec![
            EnumVariant {
                name: "Some".to_string(),
                name_span: 13..17,
                payload: Some(vec![EnumPayloadParam {
                    name: None,
                    type_: Type::Named("T".to_string()),
                }]),
            },
            EnumVariant {
                name: "None".to_string(),
                name_span: 19..23,
                payload: None,
            },
        ],
    };
    assert_eq!(enum_def.type_params, vec!["T".to_string()]);
}

#[test]
fn test_import_namespace() {
    let import = ImportStmt {
        source: "utils".to_string(),
        spec: ImportSpec::Namespace("utils".to_string()),
        type_only: false,
    };
    assert_eq!(import.source, "utils");
    match import.spec {
        ImportSpec::Namespace(name) => assert_eq!(name, "utils"),
        _ => panic!("expected namespace"),
    }
}

#[test]
fn test_import_named() {
    let import = ImportStmt {
        source: "math".to_string(),
        spec: ImportSpec::Named(vec![
            ("sin".to_string(), None),
            ("cos".to_string(), None),
            ("PI".to_string(), Some("pi".to_string())),
        ]),
        type_only: false,
    };
    match import.spec {
        ImportSpec::Named(bindings) => {
            assert_eq!(bindings.len(), 3);
            assert_eq!(bindings[0], ("sin".to_string(), None));
            assert_eq!(
                bindings[2],
                ("PI".to_string(), Some("pi".to_string()))
            );
        }
        _ => panic!("expected named"),
    }
}

#[test]
fn test_import_bare() {
    let import = ImportStmt {
        source: "polyfill".to_string(),
        spec: ImportSpec::Bare,
        type_only: false,
    };
    assert!(matches!(import.spec, ImportSpec::Bare));
}

#[test]
fn test_import_type_only() {
    let import = ImportStmt {
        source: "types".to_string(),
        spec: ImportSpec::Named(vec![("User".to_string(), None)]),
        type_only: true,
    };
    assert!(import.type_only);
}

#[test]
fn test_export_fn() {
    let export = ExportStmt {
        item: ExportItem::Fn(FnDef {
            name: "public_fn".to_string(),
            name_span: 7..16,
            params: vec![],
            return_type: None,
            type_params: vec![],
            bounds: vec![],
            is_async: false,
            body: Block {
                statements: vec![],
            },
            span: 0..20,
        }),
    };
    assert!(matches!(export.item, ExportItem::Fn(_)));
}

#[test]
fn test_export_names() {
    let export = ExportStmt {
        item: ExportItem::Names(vec![
            "foo".to_string(),
            "bar".to_string(),
        ]),
    };
    match export.item {
        ExportItem::Names(names) => assert_eq!(names, vec!["foo", "bar"]),
        _ => panic!("expected names"),
    }
}

#[test]
fn test_impl_fn_name() {
    assert_eq!(
        impl_fn_name("Area", "Rectangle", "area"),
        "impl_Area_Rectangle_area"
    );
    assert_eq!(
        impl_fn_name("Display", "User", "fmt"),
        "impl_Display_User_fmt"
    );
}

#[test]
fn test_type_alias() {
    let alias = TypeAlias {
        name: "UserID".to_string(),
        name_span: 5..11,
        type_params: vec![],
        type_: Type::Number,
    };
    assert_eq!(alias.name, "UserID");
}

#[test]
fn test_type_alias_generic() {
    let alias = TypeAlias {
        name: "Predicate".to_string(),
        name_span: 5..14,
        type_params: vec!["T".to_string()],
        type_: Type::FnSig {
            params: vec![Type::Named("T".to_string())],
            ret: Some(Box::new(Type::Boolean)),
        },
    };
    assert_eq!(alias.type_params, vec!["T".to_string()]);
}

#[test]
fn test_trait_decl() {
    let trait_decl = TraitDecl {
        name: "Display".to_string(),
        name_span: 6..13,
        type_params: vec![],
        methods: vec![TraitMethod {
            name: "fmt".to_string(),
            name_span: 15..18,
            has_self: true,
            params: vec![],
            return_type: Some(Type::String),
            is_async: false,
            span: 15..25,
        }],
        span: 0..25,
    };
    assert_eq!(trait_decl.name, "Display");
    assert_eq!(trait_decl.methods.len(), 1);
    assert!(trait_decl.methods[0].has_self);
}

#[test]
fn test_impl_decl() {
    let impl_decl = ImplDecl {
        trait_name: "Display".to_string(),
        type_name: "User".to_string(),
        methods: vec![FnDef {
            name: "fmt".to_string(),
            name_span: 10..13,
            params: vec![],
            return_type: Some(Type::String),
            type_params: vec![],
            bounds: vec![],
            is_async: false,
            body: Block {
                statements: vec![],
            },
            span: 0..20,
        }],
        span: 0..25,
        is_inherent: false,
    };
    assert_eq!(impl_decl.trait_name, "Display");
    assert_eq!(impl_decl.type_name, "User");
}

#[test]
fn test_binding_pattern_ident() {
    let pattern = BindingPattern::Ident("x".to_string());
    assert!(matches!(pattern, BindingPattern::Ident(s) if s == "x"));
}

#[test]
fn test_binding_pattern_destructure() {
    let pattern = BindingPattern::Destructure(vec![
        ("a".to_string(), None),
        ("b".to_string(), Some("c".to_string())),
    ]);
    match pattern {
        BindingPattern::Destructure(bindings) => {
            assert_eq!(bindings.len(), 2);
            assert_eq!(bindings[0], ("a".to_string(), None));
            assert_eq!(bindings[1], ("b".to_string(), Some("c".to_string())));
        }
        _ => panic!("expected destructure"),
    }
}
