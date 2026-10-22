use xulo_core::ast::*;

#[test]
fn test_enum_def_basic() {
    let enum_def = EnumDef {
        name: "Color".to_string(),
        name_span: 5..10,
        type_params: vec![],
        type_bounds: Vec::new(),
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
        attributes: Vec::new(),
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
        type_bounds: Vec::new(),
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
        attributes: Vec::new(),
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
        type_bounds: Vec::new(),
        variants: vec![
            EnumVariant {
                name: "Some".to_string(),
                name_span: 13..17,
                payload: Some(vec![EnumPayloadParam {
                    name: None,
                    type_: Type::Named("T".to_string(), Vec::new()),
                }]),
            },
            EnumVariant {
                name: "None".to_string(),
                name_span: 19..23,
                payload: None,
            },
        ],
        attributes: Vec::new(),
    };
    assert_eq!(enum_def.type_params, vec!["T".to_string()]);
}

#[test]
fn test_use_path() {
    let use_decl = UseDecl {
        is_pub: false,
        type_only: false,
        target: UseTarget::Path {
            head: PathHead::Name("utils".to_string()),
            segments: vec!["run".to_string()],
            alias: Some("run_util".to_string()),
        },
        span: 0..25,
    };
    let UseTarget::Path {
        head,
        segments,
        alias,
    } = &use_decl.target
    else {
        panic!("expected path target");
    };
    assert!(matches!(head, PathHead::Name(n) if n == "utils"));
    assert_eq!(segments, &["run".to_string()]);
    assert_eq!(alias.as_deref(), Some("run_util"));
}

#[test]
fn test_use_group_named() {
    let use_decl = UseDecl {
        is_pub: false,
        type_only: false,
        target: UseTarget::Group {
            head: PathHead::Name("math".to_string()),
            entries: vec![
                UseEntry {
                    segments: vec!["sin".to_string()],
                    alias: None,
                },
                UseEntry {
                    segments: vec!["cos".to_string()],
                    alias: None,
                },
                UseEntry {
                    segments: vec!["PI".to_string()],
                    alias: Some("pi".to_string()),
                },
            ],
        },
        span: 0..30,
    };
    let UseTarget::Group { head, entries } = &use_decl.target else {
        panic!("expected group target");
    };
    assert!(matches!(head, PathHead::Name(n) if n == "math"));
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].segments, ["sin".to_string()]);
    assert_eq!(entries[2].alias.as_deref(), Some("pi"));
}

#[test]
fn test_use_glob() {
    let use_decl = UseDecl {
        is_pub: false,
        type_only: false,
        target: UseTarget::Glob {
            head: PathHead::Name("polyfill".to_string()),
            segments: Vec::new(),
        },
        span: 0..20,
    };
    assert!(matches!(use_decl.target, UseTarget::Glob { .. }));
}

#[test]
fn test_use_type_only() {
    let use_decl = UseDecl {
        is_pub: false,
        type_only: true,
        target: UseTarget::Path {
            head: PathHead::Name("types".to_string()),
            segments: vec!["User".to_string()],
            alias: None,
        },
        span: 0..18,
    };
    assert!(use_decl.type_only);
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
            body: Block { statements: vec![] },
            span: 0..20,
            attributes: Vec::new(),
            title: None,
            abi: None,
        }),
    };
    assert!(matches!(export.item, ExportItem::Fn(_)));
}

#[test]
fn test_export_names() {
    let export = ExportStmt {
        item: ExportItem::Names(vec!["foo".to_string(), "bar".to_string()]),
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
        type_bounds: Vec::new(),
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
        type_bounds: Vec::new(),
        type_: Type::FnSig {
            params: vec![Type::Named("T".to_string(), Vec::new())],
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
        attributes: Vec::new(),
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
        type_params: vec![],
        type_bounds: vec![],
        type_args: vec![],
        methods: vec![FnDef {
            name: "fmt".to_string(),
            name_span: 10..13,
            params: vec![],
            return_type: Some(Type::String),
            type_params: vec![],
            bounds: vec![],
            is_async: false,
            body: Block { statements: vec![] },
            span: 0..20,
            attributes: Vec::new(),
            title: None,
            abi: None,
        }],
        span: 0..25,
        is_inherent: false,
        attributes: Vec::new(),
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
