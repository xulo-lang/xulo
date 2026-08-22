use xulo_core::ast::*;

#[test]
fn test_type_name_primitives() {
    assert_eq!(Type::String.name(), "string");
    assert_eq!(Type::Number.name(), "number");
    assert_eq!(Type::Boolean.name(), "boolean");
    assert_eq!(Type::Null.name(), "null");
    assert_eq!(Type::Object.name(), "object");
    assert_eq!(Type::Any.name(), "any");
}

#[test]
fn test_type_name_named() {
    assert_eq!(Type::Named("User".to_string()).name(), "User");
    assert_eq!(Type::Named("Vec".to_string()).name(), "Vec");
}

#[test]
fn test_type_name_literal() {
    assert_eq!(
        Type::Literal("active".to_string()).name(),
        "\"active\""
    );
    assert_eq!(
        Type::Literal("draft".to_string()).name(),
        "\"draft\""
    );
}

#[test]
fn test_type_name_list() {
    assert_eq!(
        Type::List(Box::new(Type::Number)).name(),
        "list<number>"
    );
    assert_eq!(
        Type::List(Box::new(Type::String)).name(),
        "list<string>"
    );
}

#[test]
fn test_type_name_nested_list() {
    assert_eq!(
        Type::List(Box::new(Type::List(Box::new(Type::Number)))).name(),
        "list<list<number>>"
    );
}

#[test]
fn test_type_name_optional() {
    assert_eq!(
        Type::Optional(Box::new(Type::String)).name(),
        "string?"
    );
    assert_eq!(
        Type::Optional(Box::new(Type::Number)).name(),
        "number?"
    );
}

#[test]
fn test_type_name_union() {
    let union = Type::Union(vec![Type::Number, Type::String]);
    assert_eq!(union.name(), "number | string");
}

#[test]
fn test_type_name_union_three() {
    let union = Type::Union(vec![Type::Number, Type::String, Type::Boolean]);
    assert_eq!(union.name(), "number | string | boolean");
}

#[test]
fn test_type_name_intersection() {
    let inter = Type::Intersection(vec![
        Type::Named("A".to_string()),
        Type::Named("B".to_string()),
    ]);
    assert_eq!(inter.name(), "A & B");
}

#[test]
fn test_type_name_object_type() {
    let obj = Type::ObjectType(vec![
        ("width".to_string(), Type::Number),
        ("height".to_string(), Type::Number),
    ]);
    assert_eq!(obj.name(), "{width: number, height: number}");
}

#[test]
fn test_type_name_object_type_single() {
    let obj = Type::ObjectType(vec![("name".to_string(), Type::String)]);
    assert_eq!(obj.name(), "{name: string}");
}

#[test]
fn test_type_name_fn_sig() {
    let sig = Type::FnSig {
        params: vec![Type::Number, Type::String],
        ret: Some(Box::new(Type::Boolean)),
    };
    assert_eq!(sig.name(), "fn(number, string): boolean");
}

#[test]
fn test_type_name_fn_sig_no_return() {
    let sig = Type::FnSig {
        params: vec![Type::Number],
        ret: None,
    };
    assert_eq!(sig.name(), "fn(number)");
}

#[test]
fn test_type_name_fn_sig_no_params() {
    let sig = Type::FnSig {
        params: vec![],
        ret: Some(Box::new(Type::Null)),
    };
    assert_eq!(sig.name(), "fn(): null");
}

#[test]
fn test_type_name_async() {
    assert_eq!(
        Type::Async(Box::new(Type::String)).name(),
        "async"
    );
}
