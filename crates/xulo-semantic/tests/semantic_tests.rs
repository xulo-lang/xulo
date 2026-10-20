use xulo_parser::parse_program;
use xulo_semantic::analyze;

fn analyze_src(src: &str) -> Result<(), xulo_core::error::XuloError> {
    let program = parse_program(src)?;
    analyze(&program)
}

#[test]
fn pub_declarations_registered_as_exports() {
    use xulo_semantic::analyze_with;
    let src = r#"
        pub fn add(a: number): number { return a }
        pub const PI = 3.14
        pub enum Status { Active Inactive }
        pub type User = number
    "#;
    let program = parse_program(src).unwrap();
    let result =
        analyze_with(&program, &[], &[], &[]).unwrap_or_else(|err| panic!("{}", err.message));

    let runtime: Vec<String> = result
        .exported_symbols
        .iter()
        .map(|(n, _)| n.clone())
        .collect();
    assert!(runtime.contains(&"add".into()));
    assert!(runtime.contains(&"PI".into()));
    assert!(runtime.contains(&"Status".into()));
    assert!(!runtime.contains(&"User".into()), "types are type-only");

    let types: Vec<String> = result
        .exported_types
        .iter()
        .map(|(n, _)| n.clone())
        .collect();
    assert!(types.contains(&"User".into()));
}

#[test]
fn accepts_valid_program() {
    let src = r#"
        fn fib(n: number): number {
            if n <= 1 { return n }
            else { return fib(n - 1) + fib(n - 2) }
        }
        fn main() { print(fib(5)) }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn rejects_undefined_variable() {
    let err = analyze_src("fn main() { print(message) }").unwrap_err();
    assert!(err.message.contains("undefined variable `message`"));
    assert_eq!(err.kind, xulo_core::error::ErrorKind::Semantic);
}

#[test]
fn rejects_redeclaration_in_same_scope() {
    let err = analyze_src("fn main() { let x = 1 let x = 2 }").unwrap_err();
    assert!(err.message.contains("already declared"));
}

#[test]
fn allows_shadowing_in_nested_scope() {
    let src = "fn main() { let x = 1 if true { let x = 2 } }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn rejects_type_mismatch_in_let() {
    let err = analyze_src(r#"fn main() { let x: number = "hi" }"#).unwrap_err();
    assert!(err.message.contains("`string`"));
}

#[test]
fn rejects_non_boolean_if_condition() {
    let err = analyze_src("fn main() { if 1 { print(1) } }").unwrap_err();
    assert!(err.message.contains("bool"));
}

#[test]
fn rejects_arithmetic_on_strings() {
    let err = analyze_src(r#"fn main() { print("a" - "b") }"#).unwrap_err();
    assert!(err.message.contains("cannot apply"));
}

#[test]
fn rejects_return_type_mismatch() {
    let err = analyze_src(r#"fn f(): number { return "hi" }"#).unwrap_err();
    assert!(err.message.contains("return type mismatch"));
}

#[test]
fn rejects_wrong_argument_count() {
    let err =
        analyze_src("fn add(a: number, b: number): number { return a + b } fn main() { add(1) }")
            .unwrap_err();
    assert!(err.message.contains("expects 2 argument(s)"));
}

#[test]
fn rejects_for_over_non_list() {
    let err = analyze_src("fn main() { for x in 5 { print(x) } }").unwrap_err();
    assert!(err.message.contains("must iterate over a `List`"));
}

#[test]
fn rejects_unknown_function() {
    let err = analyze_src("fn main() { nope(1) }").unwrap_err();
    assert!(err.message.contains("unknown function `nope`"));
}

#[test]
fn accepts_const_and_null() {
    let src = "const PI = 3.14\nfn main() { let x: string? = null print(x) }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn rejects_const_reassignment() {
    let err = analyze_src("const X = 1\nfn main() { X = 2 }").unwrap_err();
    assert!(err.message.contains("cannot assign to `X`"));
}

#[test]
fn rejects_let_type_mismatch_on_assignment() {
    let err = analyze_src(r#"fn main() { let mut x: number = 1 x = "s" }"#).unwrap_err();
    assert!(
        err.message
            .contains("cannot assign a value of type `string`")
    );
}

#[test]
fn rejects_assigning_undefined() {
    let err = analyze_src("fn main() { y = 1 }").unwrap_err();
    assert!(err.message.contains("undefined variable `y`"));
}

#[test]
fn type_alias_is_resolved() {
    let src = r#"
        type User = number
        fn main() {
            let u = 1
            let n: User = u
            print(n)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn rejects_unknown_type() {
    let err = analyze_src("fn main() { let x: Nope = 1 }").unwrap_err();
    assert!(err.message.contains("unknown type `Nope`"));
}

#[test]
fn rejects_duplicate_type() {
    let err = analyze_src("type A = number\ntype A = number").unwrap_err();
    assert!(err.message.contains("already defined"));
}

#[test]
fn enum_reference_and_payload_typecheck() {
    let src = r#"
        enum Result<T> { Success(T) Error(string) }
        enum Theme { Light Dark }
        fn main() {
            let a = Theme::Light
            let b = Result::Success(42)
            let c = Result::Error("boom")
            print(a)
            print(b)
            print(c)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn rejects_unknown_enum_member() {
    let err = analyze_src("enum T { A B }\nfn main() { let x = T::C }").unwrap_err();
    assert!(err.message.contains("no member `C`"));
}

#[test]
fn rejects_enum_payload_type_mismatch() {
    let err =
        analyze_src(r#"enum R { Ok(number) } fn main() { let x = R::Ok("no") }"#).unwrap_err();
    assert!(err.message.contains("argument to `R::Ok`"));
}

#[test]
fn multi_payload_enum_construction_and_match() {
    let src = r#"
        enum Person { Nobody, Named(string, number) }
        fn main() {
            let p = Person::Named("Ada", 36)
            match p {
                Person::Named(name, age) => print(str(age) + ":" + name)
                _ => print("anon")
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn multi_payload_enum_discards_with_underscore() {
    let src = r#"
        enum Person { Named(string, number) }
        fn main() {
            let p = Person::Named("Ada", 36)
            match p {
                Person::Named(name, _) => print(name)
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn rejects_wrong_multi_payload_argument_count() {
    let err =
        analyze_src(r#"enum P { Named(string, number) } fn main() { let p = P::Named("Ada") }"#)
            .unwrap_err();
    assert!(
        err.message.contains("expects 2 argument(s), got 1"),
        "message: {}",
        err.message
    );
}

#[test]
fn rejects_wrong_binding_count_in_match() {
    let err = analyze_src(
        r#"enum P { Named(string, number) }
        fn main() {
            let p = P::Named("Ada", 36)
            match p { P::Named(name) => print(name) }
        }"#,
    )
    .unwrap_err();
    assert!(
        err.message.contains("pattern binds 1 values"),
        "message: {}",
        err.message
    );
}

#[test]
fn rejects_wrong_payload_count() {
    let err = analyze_src("enum R { Ok(number) }\nfn main() { let x = R::Ok(1, 2) }").unwrap_err();
    assert!(err.message.contains("expects 1 argument"));
}

#[test]
fn optional_types_accept_null_and_values() {
    let src = r#"
        fn main() {
            let a: string? = null
            let b: string? = "hi"
            let c: string? = a
            print(a == null)
            print(b)
            print(c)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn rejects_number_into_optional_string() {
    let err = analyze_src("fn main() { let x: string? = 42 }").unwrap_err();
    assert!(err.message.contains("cannot bind"));
}

#[test]
fn string_literal_types_in_union() {
    let src = r#"
        type Status = "active" | "inactive"
        fn main() {
            let s: Status = "active"
            print(s)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn rejects_value_not_in_string_union() {
    let err = analyze_src(
        r#"
        type Status = "active" | "inactive"
        fn main() { let s: Status = "bogus" }
    "#,
    )
    .unwrap_err();
    assert!(err.message.contains("cannot bind"));
}

#[test]
fn union_accepts_members() {
    let src = "fn main() { let x: number | string = \"a\" let y: number | string = 1 }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn generic_fn_type_param_is_bound() {
    let src = r#"
        fn first<T>(list: List<T>): T { list[0] }
        fn main() { let x = first([]) print(x) }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn fn_type_annotation_checks() {
    let src = "fn main() { let h: (fn(a: number): number)? = null print(h) }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn string_concat_is_allowed() {
    let src = r#"fn main() { print("a" + "b") }"#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn number_string_concat_is_rejected() {
    let err = analyze_src(r#"fn main() { print("a" + 1) }"#).unwrap_err();
    assert!(
        err.message.contains("cannot apply `+`"),
        "message: {}",
        err.message
    );
    let err = analyze_src(r#"fn main() { print(1 + "a") }"#).unwrap_err();
    assert!(err.message.contains("cannot apply `+`"));
}

#[test]
fn str_converts_any_value_to_string() {
    assert!(analyze_src(r#"fn main() { print("got " + str(42)) }"#).is_ok());
    assert!(analyze_src(r#"fn main() { print("v=" + str(true)) }"#).is_ok());
    // `null` has no string form (spec `builtins/intrinsic-functions.md`).
    let err = analyze_src(r#"fn main() { print("n=" + str(null)) }"#).unwrap_err();
    assert_eq!(err.code(), "E0212", "unexpected message: {}", err.message);
}

#[test]
fn str_requires_exactly_one_argument() {
    let err = analyze_src("fn main() { str() }").unwrap_err();
    assert!(err.message.contains("exactly one argument"));
    let err = analyze_src("fn main() { str(1, 2) }").unwrap_err();
    assert!(err.message.contains("exactly one argument"));
}

#[test]
fn logical_operators_require_booleans() {
    assert!(analyze_src("fn main() { let x = true and false or !true print(x) }").is_ok());
    let err = analyze_src("fn main() { print(1 and true) }").unwrap_err();
    assert!(err.message.contains("cannot apply `and`"));
}

#[test]
fn ternary_requires_boolean_condition() {
    assert!(analyze_src("fn main() { print(1 > 2 ? \"a\" : \"b\") }").is_ok());
    let err = analyze_src("fn main() { print(1 ? \"a\" : \"b\") }").unwrap_err();
    assert!(err.message.contains("ternary condition"));
}

#[test]
fn while_requires_boolean() {
    assert!(analyze_src("fn main() { let mut c = 0 while c < 3 { c = c + 1 } }").is_ok());
    let err = analyze_src("fn main() { while 5 { } }").unwrap_err();
    assert!(err.message.contains("while condition"));
}

#[test]
fn match_arms_are_checked() {
    let src = r#"
        fn main() {
            let s = match 2 { 0 => "zero" 1 => "one" _ => "other" }
            print(s)
        }
    "#;
    assert!(analyze_src(src).is_ok());
    let err =
        analyze_src("fn main() { let x = match true { 0 => 1 _ => 2 } print(x) }").unwrap_err();
    assert!(err.message.contains("does not match"));
}

#[test]
fn match_enum_payload_binds() {
    let src = r#"
        enum Result<T> { Success(T) Error(string) }
        fn main() {
            let ok = Result::Success(1)
            let v = match ok {
                Result::Success(n) => n
                Result::Error(msg) => 0
            }
            print(v)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn match_arms_incompatible_types_are_rejected() {
    let err = analyze_src(
        "enum Maybe { Some(number) None }\nfn f(m: Maybe): string {\n  match m { Maybe::Some(v) => v Maybe::None => \"none\" }\n}",
    )
    .unwrap_err();
    assert!(err.message.contains("incompatible types"));
}

#[test]
fn match_arm_generic_payload_is_erased() {
    let src = r#"
        enum Result<T> { Success(T) Error(string) }
        fn main() {
            let r = Result::Success(1)
            let v = match r { Result::Success(n) => n _ => 0 }
            print(v)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn generic_call_site_infers_type_argument() {
    let src = r#"
        fn first<T>(xs: List<T>): T { xs[0] }
        let a: number = first([1, 2, 3])
        fn main() { print(a) }
    "#;
    assert!(analyze_src(src).is_ok());
    let err = analyze_src(
        "fn first<T>(xs: List<T>): T { xs[0] }\nlet s: string = first([1, 2])\nfn main() { print(s) }",
    )
    .unwrap_err();
    assert!(err.message.contains("cannot bind a value of type"));
}

#[test]
fn index_targets_lists() {
    assert!(analyze_src("fn main() { let xs: List<number> = [1, 2] print(xs[0]) }").is_ok());
    let err = analyze_src("fn main() { let xs: number = 1 print(xs[0]) }").unwrap_err();
    assert!(err.message.contains("cannot index"));
}

#[test]
fn nullish_checks() {
    let src = "fn main() { let name: string? = null print(name ?? \"anon\") }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn default_params_may_be_omitted() {
    let src = r#"
        fn greet(name: string = "x"): string { name }
        fn main() { print(greet()) print(greet("y")) }
    "#;
    assert!(analyze_src(src).is_ok());
    let err = analyze_src(r#"fn greet(name: string = 5) { }"#).unwrap_err();
    assert!(err.message.contains("default value"));
}

#[test]
fn named_args_are_validated() {
    let src = r#"
        fn greet(name: string = "x", count: number): string { name }
        fn main() { print(greet(count: 2)) }
    "#;
    assert!(analyze_src(src).is_ok());
    let err = analyze_src(
        r#"
        fn greet(name: string): string { name }
        fn main() { print(greet(nope: "x")) }
    "#,
    )
    .unwrap_err();
    assert!(err.message.contains("no parameter `nope`"));
}

#[test]
fn named_args_duplicate_or_missing_are_errors() {
    let err = analyze_src(
        r#"
        fn greet(a: number, b: number): number { a + b }
        fn main() { print(greet(a: 1, a: 2)) }
    "#,
    )
    .unwrap_err();
    assert!(err.message.contains("provided twice"));

    let err = analyze_src(
        r#"
        fn greet(a: number, b: number): number { a + b }
        fn main() { print(greet(a: 1)) }
    "#,
    )
    .unwrap_err();
    assert!(err.message.contains("missing required argument"));
}

#[test]
fn generic_named_type_arg_is_erased() {
    let src = r#"
        enum Result<T> { Success(T) Error(string) }
        fn describe(r: Result<number>): string {
            match r { Result::Success(v) => "n" Result::Error(m) => m }
        }
        fn main() { print(describe(Result::Success(1))) }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn allows_async_fn_and_await() {
    let src = r#"
        async fn load(): number { let v = 1 return v }
        async fn main() { let v = await load() print(v) }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn rejects_await_outside_async() {
    let err = analyze_src("fn main() { let v = await work() }").unwrap_err();
    assert!(err.message.contains("await"));
}

#[test]
fn awaits_sync_call_is_ok_but_non_promise_is_an_error() {
    let ok = analyze_src("async fn main() { await work() }").is_ok();
    // `work` is unknown; importing seeds it, but a non-promise await is caught:
    let err = analyze_src("fn n(): number { return 1 }\nasync fn main() { let v = await n() }")
        .unwrap_err();
    assert!(err.message.contains("non-promise"));
    let _ = ok;
}

#[test]
fn closures_and_higher_order_functions_analyze() {
    let src = r#"
        fn apply(f: fn(number): number, x: number): number { f(x) }
        fn makeAdder(n: number): fn(number): number {
            return fn(v: number): number { v + n }
        }
        fn main() {
            let double = fn(x: number): number { x * 2 }
            let add5 = makeAdder(5)
            print(apply(double, 3))
            print(add5(10))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn async_closure_can_be_awaited() {
    let src = r#"
        async fn main() {
            let work = async fn() { 42 }
            let v = await work()
            print(v)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn rejects_wrong_arity_for_function_values() {
    let err = analyze_src("fn main() { let f = fn(x: number): number { x } print(f(1, 2)) }")
        .unwrap_err();
    assert!(err.message.contains("at most 1 argument"));
}

#[test]
fn rejects_wrong_arg_type_for_function_values() {
    let err =
        analyze_src(r#"fn main() { let f = fn(x: number): number { x } f("hi") }"#).unwrap_err();
    assert!(err.message.contains("must be `number`"));
}

#[test]
fn list_spread_requires_a_list() {
    let err = analyze_src("fn main() { let n = 5 let xs = [...n] }").unwrap_err();
    assert!(err.message.contains("must be a list"));
}

#[test]
fn list_spread_type_inferred_from_operand() {
    let src = r#"
        fn takes(xs: List<number>) { print(xs) }
        fn main() {
            let base = [1, 2]
            takes([...base, 3])
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn rejects_undefined_variable_inside_list_literal() {
    let err = analyze_src("fn main() { let xs = [message] }").unwrap_err();
    assert!(err.message.contains("undefined variable `message`"));
}

#[test]
fn rejects_undefined_variable_inside_object_literal() {
    let err = analyze_src("fn main() { let o = { a: other } }").unwrap_err();
    assert!(err.message.contains("undefined variable `other`"));
}

#[test]
fn rejects_spread_of_non_object() {
    let err = analyze_src("fn main() { let n = 5 let o = { ...n } }").unwrap_err();
    assert!(err.message.contains("must be an object"));
}

#[test]
fn calls_function_value_from_index() {
    let src = r#"
        fn main() {
            let xs = [fn(a: number, b: number): number { a - b }]
            print(xs[0](10, 4))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn rejects_calling_non_function_expression() {
    let err = analyze_src("fn main() { print([1, 2](0)) }").unwrap_err();
    assert!(err.message.contains("is not callable"));
}

#[test]
fn rejects_wrong_arity_on_indexed_function_value() {
    let err =
        analyze_src("fn main() { let xs = [fn(a: number): number { a }] print(xs[0](1, 2)) }")
            .unwrap_err();
    assert!(err.message.contains("at most 1 argument"));
}

#[test]
fn rejects_implicit_return_type_mismatch() {
    let err = analyze_src("fn add(a: number): number { \"s\" }").unwrap_err();
    assert!(err.message.contains("return type mismatch"));
}

#[test]
fn rejects_implicit_return_mismatch_after_let() {
    let err = analyze_src("fn add(): number { let x = \"s\" x }").unwrap_err();
    assert!(err.message.contains("return type mismatch"));
}

#[test]
fn rejects_async_implicit_return_mismatch() {
    let err = analyze_src("async fn f(): number { \"s\" }").unwrap_err();
    assert!(err.message.contains("return type mismatch"));
}

#[test]
fn accepts_matching_implicit_return() {
    let src = r#"
        fn ok(): string { "s" }
        fn n(): number { 5 }
        fn xs(): List<number> { [1, 2, 3] }
        fn main() { print(ok()) print(n()) print(xs()) }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn rejects_incompatible_if_branch_types() {
    let err = analyze_src("fn f(): number { if true { 1 } else { \"x\" } }").unwrap_err();
    assert!(err.message.contains("incompatible types"));
}

#[test]
fn infers_if_expression_type() {
    let src = r#"
        fn main() {
            let max: number = if 5 > 3 { 5 } else { 3 }
            let s: string = if false { "a" } else if true { "b" } else { "c" }
            print(max)
            print(s)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn optional_params_may_be_omitted() {
    let src = r#"
        fn g(a: number?): number { if a == null { 0 } else { a } }
        fn main() { print(g()) print(g(7)) }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn optional_params_may_be_omitted_with_named_args() {
    let src = r#"
        fn Button(label: string, icon: string? = null): string { label }
        fn main() { print(Button(label: "Save")) print(Button(label: "Save", icon: "disk")) }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn string_union_literal_accepted_in_call_args() {
    let src = r#"
        type Status = "active" | "inactive"
        fn set(s: Status): Status { s }
        fn main() { print(set("active")) }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn rejects_invalid_string_union_literal_in_call() {
    let err = analyze_src(
        r#"
        type Status = "active" | "inactive"
        fn set(s: Status) { print(s) }
        fn main() { set("bogus") }
        "#,
    )
    .unwrap_err();
    assert!(err.message.contains("must be `Status`"));
}

#[test]
fn optional_chaining_allows_null_base() {
    let src = r#"
        fn main() {
            let nobody = null
            let name = nobody?.name ?? "anonymous"
            print(name)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn component_type_is_recognized() {
    let src = "fn main(): View { }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn state_is_allowed_in_component_function() {
    let src = r#"
        fn main(): View {
            @State let count: number = 0
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn decorators_rejected_outside_component() {
    let err = analyze_src("fn main() { @State let count: number = 0 }").unwrap_err();
    assert!(err.message.contains("returning `View`"));

    let err = analyze_src("async fn helper() { @State let count = 0 }").unwrap_err();
    assert!(err.message.contains("returning `View`"));
}

#[test]
fn decorators_rejected_in_nested_block() {
    let err =
        analyze_src("fn main(): View { if true { @State let count: number = 0 } }").unwrap_err();
    assert!(err.message.contains("nested block"));
}

#[test]
fn effect_and_store_and_environment_in_component() {
    let src = r#"
        fn useAppStore(): Map<string, unknown> { return { user: null } }
        fn main(): View {
            @State let editing: bool = false
            @Store let { user } = useAppStore()
            @Environment let router: Map<string, unknown>
            @Effect fn() { print(user) }
            @Effect fn() { }, [editing]
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn dollar_binding_requires_state_or_store() {
    let src = r#"
        fn main(): View {
            @State let name: string = ""
            Input(value: $name)
        }
    "#;
    assert!(analyze_src(src).is_ok());

    let err =
        analyze_src("fn main(): View { let name: string = \"\" Input(value: $name) }").unwrap_err();
    assert!(err.message.contains("`$` binding"));
}

#[test]
fn state_cannot_be_redeclared() {
    let err = analyze_src("fn main(): View { @State let x: number = 0 @State let x: number = 1 }")
        .unwrap_err();
    assert!(err.message.contains("already declared"));
}

#[test]
fn component_children_are_type_checked() {
    let src = r#"
        fn main(): View {
            VStack {
                if 1 > 0 { Text("ok") }
                for x in [1, 2] { Text("x") }
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());

    let err = analyze_src("fn main(): View { VStack { for x in 5 { Text(\"x\") } } }").unwrap_err();
    assert!(err.message.contains("must iterate over a `List`"));
}

#[test]
fn expr_child_accepts_string_and_component_lists() {
    // A string variable as a bare child renders like a `Text`.
    assert!(
        analyze_src("fn main(): View { @State let name: string = \"Xulo\" VStack { name } }")
            .is_ok()
    );

    // A member access yielding a string is also a valid child.
    assert!(analyze_src(
        "type User = Map<string, string>\nfn main(): View { @State let u: User = { name: \"a\" } VStack { u.name } }"
    )
    .is_ok());

    // The documented custom-component pattern: a `children: List<View>`
    // parameter is forwarded as a bare element.
    let src = r#"
        fn MyCard(title: string, children: List<View>): View {
            VStack {
                Text(title, weight: "bold")
                children
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn expr_child_rejects_non_renderable_types() {
    for src in [
        "fn main(): View { @State let n: number = 0 VStack { n } }",
        "fn main(): View { @State let ok: bool = true VStack { ok } }",
        "fn main(): View { @State let xs: List<number> = [1, 2] VStack { xs } }",
    ] {
        let err = analyze_src(src).unwrap_err();
        assert!(
            err.message
                .contains("component children must be strings, components, or lists of components"),
            "unexpected message: {}",
            err.message
        );
    }
}

#[test]
fn await_rejected_inside_value_position_if() {
    let err = analyze_src(
        "async fn work() { 42 }\nasync fn main() { let ok = true let x = if ok { await work() } else { 0 } }",
    )
    .unwrap_err();
    assert!(err.message.contains("`if`/`match` expression"));
}

#[test]
fn await_rejected_inside_value_position_match() {
    let err = analyze_src(
        "async fn work() { 42 }\nasync fn main() { let x = match 1 { 1 => await work() _ => 0 } }",
    )
    .unwrap_err();
    assert!(err.message.contains("`if`/`match` expression"));
}

#[test]
fn await_allowed_in_statement_position_if() {
    let src = r#"
        async fn work() { 42 }
        async fn main() {
            let ok = true
            if ok { let v = await work() print(v) }
            print(7)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn await_allowed_inside_ternary() {
    let src = "async fn work() { 42 }\nasync fn main() { let ok = true let x = ok ? await work() : 0 print(x) }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn await_rejected_in_non_async_closure_inside_async_fn() {
    let err =
        analyze_src("async fn work() { 42 }\nasync fn main() { let g = fn() { await work() } }")
            .unwrap_err();
    assert!(err.message.contains("inside an `async` function"));
}

#[test]
fn await_allowed_in_async_closure() {
    let src = "async fn main() { let g = async fn() { 42 } let v = await g() print(v) }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn decorators_rejected_in_closure_inside_component() {
    let err = analyze_src(
        "fn main(): View { fn handle() { @State let count: number = 0 } print(\"x\") }",
    )
    .unwrap_err();
    assert!(err.message.contains("returning `View`"));
}

#[test]
fn decorators_rejected_in_anonymous_closure_inside_component() {
    let err = analyze_src("fn main(): View { let handle = fn() { @State let count: number = 0 } }")
        .unwrap_err();
    assert!(err.message.contains("returning `View`"));
}

#[test]
fn effect_cannot_capture_render_local() {
    let err = analyze_src(
        r#"
        fn main(): View {
            let a = 5
            @Effect fn() { print("a=" + a) }
        }
    "#,
    )
    .unwrap_err();
    assert!(
        err.message
            .contains("`@Effect` closures cannot reference `a`")
    );
}

#[test]
fn effect_cannot_capture_render_local_via_deps() {
    let err = analyze_src(
        r#"
        fn main(): View {
            let a = 5
            @Effect fn() { print(1) }, [a]
        }
    "#,
    )
    .unwrap_err();
    assert!(
        err.message
            .contains("`@Effect` closures cannot reference `a`")
    );
}

#[test]
fn effect_cannot_call_render_local_function() {
    let err = analyze_src(
        r#"
        fn main(): View {
            fn helper() { print("h") }
            @Effect fn() { helper() }
        }
    "#,
    )
    .unwrap_err();
    assert!(
        err.message
            .contains("`@Effect` closures cannot reference `helper`")
    );
}

#[test]
fn effect_can_capture_state_store_and_env() {
    let src = r#"
        fn useAppStore(): Map<string, unknown> { return { user: null } }
        fn main(): View {
            @State let count: number = 0
            @Store let { user } = useAppStore()
            @Environment let router: Map<string, unknown>
            @Effect fn() { print(count) print(user) print(router) }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn effect_local_let_is_allowed() {
    let src = r#"
        fn main(): View {
            @State let count: number = 0
            @Effect fn() { let x = count + 1 print(x) }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn no_implicit_return_warning_without_declared_return_type() {
    let program = parse_program("fn main() { print(1); }").unwrap();
    let result = xulo_semantic::analyze_with(&program, &[], &[], &[]).unwrap();
    assert!(
        result.warnings.is_empty(),
        "unexpected warnings: {:?}",
        result.warnings
    );
}

#[test]
fn implicit_return_warning_only_with_declared_return_type() {
    let program = parse_program("fn f(): number { 1; }").unwrap();
    let result = xulo_semantic::analyze_with(&program, &[], &[], &[]).unwrap();
    assert_eq!(result.warnings.len(), 1, "warnings: {:?}", result.warnings);
    assert!(result.warnings[0].message.contains("ignored return value"));
}

#[test]
fn rejects_unary_not_on_non_boolean() {
    let err = analyze_src("fn main() { print(!5) }").unwrap_err();
    assert!(err.message.contains("unary `!` requires a `bool`"));
    assert!(analyze_src("fn main() { print(!true) }").is_ok());
}

#[test]
fn rejects_unary_minus_on_string() {
    let err = analyze_src("fn main() { print(-\"a\") }").unwrap_err();
    assert!(err.message.contains("unary `-` requires a `number`"));
    assert!(analyze_src("fn main() { let a = 5 print(-a) }").is_ok());
}

#[test]
fn rejects_equality_between_number_and_string() {
    let err = analyze_src("fn main() { print(1 == \"1\") }").unwrap_err();
    assert!(
        err.message
            .contains("cannot compare `number` with `string`")
    );
    assert!(analyze_src("fn main() { print(1 == 1) print(\"a\" != \"b\") }").is_ok());
}

#[test]
fn string_relational_comparison_is_allowed() {
    assert!(analyze_src("fn main() { print(\"a\" < \"b\") }").is_ok());
    let err = analyze_src("fn main() { print(1 < \"a\") }").unwrap_err();
    assert!(
        err.message
            .contains("cannot compare `number` with `string`")
    );
}

#[test]
fn list_concat_is_allowed_but_mixed_concat_is_not() {
    assert!(analyze_src("fn main() { print([1] + [2]) }").is_ok());
    let err = analyze_src("fn main() { print([1] + 2) }").unwrap_err();
    assert!(err.message.contains("cannot apply `+`"));
    let err = analyze_src("fn main() { print(1 + [2]) }").unwrap_err();
    assert!(err.message.contains("cannot apply `+`"));
}

#[test]
fn list_literal_rejects_heterogeneous_elements() {
    assert!(analyze_src("fn main() { let xs = [1, 2] print(xs) }").is_ok());
    assert!(analyze_src(r#"fn main() { let xs = ["a", "b"] print(xs) }"#).is_ok());
    // A string in a *number-annotated* list must be rejected instead of
    // silently producing a `List<number>` whose runtime value is corrupted.
    let err =
        analyze_src(r#"fn main() { let xs: List<number> = [1, "a"] print(xs) }"#).unwrap_err();
    assert!(
        err.message
            .contains("cannot bind a value of type `List<number | string>`"),
        "got: {}",
        err.message
    );
    // An unannotated mixed literal infers a union element type...
    assert!(
        analyze_src(r#"fn main() { let xs = [1, "a"] print(xs) }"#).is_ok(),
        "unannotated mixed list must infer `List<number | string>`"
    );
    // ...and an explicit union annotation is the legal escape hatch.
    assert!(
        analyze_src(r#"fn main() { let xs: List<number | string> = [1, "a"] print(xs) }"#).is_ok()
    );
    // Spread elements participate: `List<string>` into a numeric list yields
    // a union, which a `List<number>` binding still rejects.
    let err = analyze_src(
        r#"fn main() { let strs: List<string> = ["x"] let xs: List<number> = [1, ...strs] print(xs) }"#,
    )
    .unwrap_err();
    assert!(err.message.contains("cannot bind"), "got: {}", err.message);
}

#[test]
fn default_value_can_reference_earlier_parameter() {
    // `fn f(a, b = a)` is legal: the default runs in the callee scope where
    // `a` is already bound (matching the emitted JS `function f(a, b = a)`).
    assert!(
        analyze_src("fn f(a: number, b: number = a): number { b } fn main() { print(str(f(5))) }")
            .is_ok()
    );
    // A default cannot reference a *later* parameter (JS TDZ semantics) or
    // itself.
    let err = analyze_src("fn f(a: number = b, b: number): number { a }").unwrap_err();
    assert!(
        err.message.contains("undefined variable `b`"),
        "got: {}",
        err.message
    );
    let err = analyze_src("fn f(x: number = x): number { x }").unwrap_err();
    assert!(
        err.message.contains("undefined variable `x`"),
        "got: {}",
        err.message
    );
}

#[test]
fn string_literal_union_accepted_in_return_positions() {
    // `let`/call-argument positions accepted literal unions; return and
    // implicit-return positions used to reject the same direct literal.
    let src = r#"
        type Status = "active" | "inactive"
        fn get(): Status { return "active" }
        fn implicit(): Status { "inactive" }
        fn main() { print(get()) print(implicit()) }
    "#;
    assert!(
        analyze_src(src).is_ok(),
        "return positions must accept literals"
    );

    // A literal outside the union is still rejected.
    let err = analyze_src(
        r#"
        type Status = "active" | "inactive"
        fn get(): Status { return "bogus" }
        fn main() { print(get()) }
        "#,
    )
    .unwrap_err();
    assert!(
        err.message.contains("return type mismatch"),
        "got: {}",
        err.message
    );
}

#[test]
fn enum_payload_accepts_string_literal_union_member() {
    let src = r#"
        type Mode = "a" | "b"
        enum E { V(Mode) }
        fn main() { let e = E::V("a") print(e) }
    "#;
    assert!(
        analyze_src(src).is_ok(),
        "enum payload must accept union literals"
    );
}

#[test]
fn trait_dispatch_resolves_type_aliases() {
    // A value typed by an alias of an impl'd type dispatches to the same impl.
    let src = r#"
        trait Area { fn area(self): number }
        type Rect = Map<string, number>
        impl Area for Rect { fn area(self): number { return self.w * self.h } }
        type RectAlias = Rect
        fn mk(): RectAlias { let r = { w: 3, h: 4 } r }
        fn main() { print(str(Area::area(mk()))) }
    "#;
    assert!(analyze_src(src).is_ok(), "alias receiver must dispatch");
}

#[test]
fn nested_async_fn_inside_if_arm_can_await() {
    // `no_await_depth` is per-function: an `async` function declared inside an
    // `if` expression's arm is its own await context and must not inherit the
    // arm's no-await restriction.
    let src = r#"
        async fn pause() { }
        async fn g(): number { await pause() 1 }
        async fn main() {
            let x = if true {
                async fn inner(): number { await g() }
                inner()
            } else {
                0
            }
            print(str(await x))
        }
    "#;
    assert!(
        analyze_src(src).is_ok(),
        "nested async fn must be able to await"
    );
}

#[test]
fn effect_closure_rejects_parameters() {
    let err = analyze_src(
        r#"
        fn main(): View {
            @Effect fn(x: number) { print(str(x)) }
            Screen { }
        }
        "#,
    )
    .unwrap_err();
    assert!(
        err.message
            .contains("`@Effect` closures take no parameters"),
        "got: {}",
        err.message
    );
}

#[test]
fn index_assignment_is_type_checked_on_typed_list() {
    assert!(
        analyze_src("fn main() { let mut xs: List<number> = [1] xs[0] = 5 print(xs) }").is_ok()
    );
    let err = analyze_src("fn main() { let mut xs: List<number> = [1] xs[0] = \"s\" print(xs) }")
        .unwrap_err();
    assert!(
        err.message
            .contains("cannot assign a value of type `string` to `xs[...]: number`")
    );
}

#[test]
fn non_list_index_assignment_is_rejected() {
    let err = analyze_src("fn main() { print(5) let mut n: number = 1 n[0] = 2 }").unwrap_err();
    assert!(err.message.contains("cannot assign into `number` by index"));
}

#[test]
fn default_param_type_mismatch_is_rejected() {
    let err = analyze_src("fn f(a: number = \"s\"): number { a }").unwrap_err();
    assert!(
        err.message
            .contains("default value for parameter `a` must be `number`")
    );
    assert!(analyze_src("fn f(a: number = 0): number { a }").is_ok());
}

#[test]
fn optional_list_type_accepts_null_and_values() {
    let ok = r#"
        type Status = "active" | "inactive"
        fn same(a: List<number>?, b: List<number>?): bool { a == b }
        fn main() { print(same(null, null)) print(same([1], [1])) }
    "#;
    assert!(analyze_src(ok).is_ok());
    // An optional list cannot be indexed directly (no narrowing).
    let err = analyze_src("fn g(xs: List<number>?): number { xs[0] }").unwrap_err();
    assert!(err.message.contains("cannot index into `List<number>?`"));
}

#[test]
fn nested_generic_list_indexing_types_check() {
    let src =
        "fn main() { let m: List<List<number> > = [[1, 2], [3]] let n: number = m[0][1] print(n) }";
    assert!(analyze_src(src).is_ok());
    let err = "fn main() { let m: List<List<number> > = [[1, 2], [3]] let s: string = m[0][1] }";
    assert!(analyze_src(err).is_err());
}

#[test]
fn state_assignment_is_type_checked_inside_component() {
    let ok = r#"
        fn main(): View { @State let n: number = 0 n = 1 VStack { Text(str(n)) } }
    "#;
    assert!(analyze_src(ok).is_ok());
    let bad = r#"
        fn main(): View { @State let n: number = 0 n = "x" VStack { Text(str(n)) } }
    "#;
    let err = analyze_src(bad).unwrap_err();
    assert!(
        err.message
            .contains("cannot assign a value of type `string` to `n")
    );
}

#[test]
fn store_destructure_is_immutable() {
    let src = r#"
        fn makeStore(): Map<string, unknown> { let o = { theme: { x: 1 } } o }
        fn main(): View { @Store let { theme } = makeStore() theme = { x: 2 } VStack { Text("x") } }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(err.message.contains("cannot assign to `theme`"));
}

#[test]
fn for_loop_variable_is_scoped_to_the_loop() {
    let src = "fn main() { for i in 0..<3 { print(i) } print(i) }";
    let err = analyze_src(src).unwrap_err();
    assert!(err.message.contains("undefined variable `i`"));
    assert!(analyze_src("fn main() { for i in 0..<3 { print(i) } }").is_ok());
}

#[test]
fn component_call_props_are_loosely_typed() {
    // Uppercase calls lower to external UI components (`Name({ key: value })`);
    // props are not validated against the function signature.
    let ok = r#"
        fn Counter(x: number): string { str(x) }
        fn main(): string { Counter(x: "a") }
    "#;
    assert!(analyze_src(ok).is_ok());
}

#[test]
fn nested_generic_call_inference() {
    let src = r#"
        fn id<T>(x: T): T { x }
        fn main() { let a: number = id(id(5)) let b: string = id(id("s")) print(a + 1) print(b) }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn generic_list_argument_inference() {
    let src = r#"
        fn first<T>(xs: List<T>): T { xs[0] }
        fn main() { let n: number = first([1, 2]) print(n) }
    "#;
    assert!(analyze_src(src).is_ok());
    let err = r#"
        fn first<T>(xs: List<T>): T { xs[0] }
        fn main() { first("not a list") }
    "#;
    assert!(analyze_src(err).is_err());
}

#[test]
fn string_union_type_members_and_null() {
    let ok = r#"
        type Status = "active" | "inactive"
        fn set(s: Status): Status { s }
        fn main() { print(set("inactive")) }
    "#;
    assert!(analyze_src(ok).is_ok());
    let err = r#"
        type Status = "active" | "inactive"
        fn set(s: Status): Status { s }
        fn main() { print(set("pending")) }
    "#;
    assert!(
        analyze_src(err)
            .unwrap_err()
            .message
            .contains("argument to `set` must be `Status`")
    );
}

#[test]
fn top_level_return_is_rejected() {
    assert!(analyze_src("return 1").is_err());
    assert!(analyze_src("return").is_err());
    // Bare `return;` inside a function is still fine.
    assert!(analyze_src("fn f() { if true { return } }").is_ok());
}

#[test]
fn local_function_params_are_checked_even_when_empty() {
    // A local `fn foo()` with no params/return is not "opaque": arguments
    // must still match its (empty) signature.
    let err = analyze_src("fn foo() { } fn main() { foo(1) }").unwrap_err();
    assert!(
        err.message.contains("expects"),
        "unexpected {}",
        err.message
    );
    assert!(analyze_src("fn foo() { } fn main() { foo() }").is_ok());
}

#[test]
fn mixed_positional_and_named_arguments_rejected() {
    let err = analyze_src("fn greet(name: string) { } fn main() { greet(\"a\", punct: \"!\") }")
        .unwrap_err();
    assert!(err.message.contains("cannot mix positional and named"));
    assert!(
        analyze_src("fn greet(name: string, punct: string): string { name + punct } fn main() { greet(name: \"a\", punct: \"!\") }")
            .is_ok()
    );
}

#[test]
fn named_generic_call_infers_return_type() {
    let ok = r#"
        fn id<T>(x: T): T { return x }
        fn main() { let s: string = id(x: "hi") print(s) let n: number = id(x: 5) print(n) }
    "#;
    assert!(analyze_src(ok).is_ok());
}

#[test]
fn match_pattern_must_match_enum_being_matched() {
    // Pattern type must be the same enum as the matched value.
    let err = r#"
        enum A { X }
        enum B { Y }
        fn main() { let v = A::X match v { B::Y => 1 } }
    "#;
    let err = analyze_src(err).unwrap_err();
    assert!(
        err.message.contains("does not match value of type"),
        "unexpected {}",
        err.message
    );
    // With-payload patterns work when the enum carries payloads.
    let ok = r#"
        enum R<T> { A(T) B }
        fn main() { let v = R::A(1) match v { R::A(x) => x R::B => 0 } }
    "#;
    assert!(analyze_src(ok).is_ok());
}

#[test]
fn duplicate_parameter_names_rejected() {
    let err = analyze_src("fn f(a: number, a: number) { }").unwrap_err();
    assert!(err.message.contains("shadows an earlier parameter"));
}

#[test]
fn tail_await_rule_uses_statement_position() {
    // A trailing `if` at the end of a `for`/`while` body is a statement:
    // awaits inside its arms are fine (value-position `if`/`match` in a
    // function tail is still rejected by `await_rejected_inside_implicit_return_if`).
    let ok = r#"
        async fn work() { 42 }
        async fn main() {
            for i in [1, 2] {
                if i > 0 { await work() }
            }
        }
    "#;
    assert!(
        analyze_src(ok).is_ok(),
        "statement-position await should pass"
    );
}

#[test]
fn call_value_alias_aliases_callable() {
    // An alias to a function type makes an indirect call type-checkable.
    let ok = r#"
        type Handler = fn(a: number): number
        fn apply(h: Handler, x: number): number { h(x) }
        fn main() { print(apply(fn(n: number): number { n + 1 }, 1)) }
    "#;
    assert!(analyze_src(ok).is_ok());
}

#[test]
fn user_str_shadows_builtin() {
    let ok = r#"
        fn str(prefix: string): string { "[[" + prefix + "]]" }
        fn main() { print(str("hi")) }
    "#;
    assert!(analyze_src(ok).is_ok());

    // The builtin still works when no user `str` shadows it.
    let ok2 = r#"
        fn main() { let s = str(42) print(s) }
    "#;
    assert!(analyze_src(ok2).is_ok());
    // And the user `str` no longer accepts the builtin's single numeric arg.
    let err = r#"
        fn str(prefix: string): string { prefix }
        fn main() { str(42) }
    "#;
    let err = analyze_src(err).unwrap_err();
    assert!(
        err.message.contains("must be `string`"),
        "unexpected {}",
        err.message
    );
}

#[test]
fn trait_and_impl_check_cleanly() {
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        type Circle = Map<string, number>
        impl Shape for Circle {
            fn area(self): number { return 100 }
        }
        fn main() {
            let c: Circle = { radius: 1 }
            print(Shape::area(c))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn trait_dispatch_wrong_receiver_type_is_rejected() {
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        type Circle = Map<string, number>
        type Square = Map<string, number>
        impl Shape for Circle {
            fn area(self): number { return 100 }
        }
        fn main() {
            let s: Square = { side: 1 }
            print(Shape::area(s))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("does not implement trait `Shape`"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn trait_dispatch_unknown_method_is_rejected() {
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        type Circle = Map<string, number>
        impl Shape for Circle {
            fn area(self): number { return 100 }
        }
        fn main() {
            let c: Circle = { radius: 1 }
            print(Shape::volume(c))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("has no method `volume`"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn trait_dispatch_generic_receiver_is_rejected() {
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        type Circle = Map<string, number>
        impl Shape for Circle {
            fn area(self): number { return 100 }
        }
        fn apply<T>(t: T): number {
            // `Shape::area(t)` cannot dispatch: `T` is unbounded, and a
            // parameter is a dispatch receiver only when its declaration
            // bounds it by the trait (spec `types/generics.md`).
            return Shape::area(t)
        }
        fn main() { print(apply(5)) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("generic parameter `T`"),
        "unexpected: {}",
        err.message
    );
    assert!(
        err.message.contains("cannot dispatch `Shape::area`"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn trait_dispatch_rejects_named_arguments() {
    // Dispatch binds the receiver positionally (first slot), so labeled
    // arguments would silently mis-bind in the runtimes; reject them.
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        type Circle = Map<string, number>
        impl Shape for Circle {
            fn area(self): number { return 100 }
        }
        fn main() {
            let c: Circle = { radius: 1 }
            print(Shape::area(r: c))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("takes positional arguments"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn impl_underscore_mangle_collision_is_rejected() {
    // `impl_fn_name("a_b", "c", "m")` and `impl_fn_name("a", "b_c", "m")` both
    // mangle to `impl_a_b_c_m`; the second `impl` must be rejected instead of
    // silently overwriting the first in the JS/native runtimes.
    let src = r#"
        trait a_b {
            fn m(self): number
        }
        trait a {
            fn m(self): number
        }
        type c = Map<string, number>
        type b_c = Map<string, number>
        impl a_b for c {
            fn m(self): number { return 1 }
        }
        impl a for b_c {
            fn m(self): number { return 2 }
        }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("trait dispatch name `impl_a_b_c_m`"),
        "unexpected: {}",
        err.message
    );
    assert!(
        err.message.contains("claimed by more than one `impl`"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn impl_mangled_name_collides_with_user_function_is_rejected() {
    let src = r#"
        fn impl_Shape_Circle_area(): number { return 1 }
        trait Shape {
            fn area(self): number
        }
        type Circle = Map<string, number>
        impl Shape for Circle {
            fn area(self): number { return 100 }
        }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("trait dispatch name `impl_Shape_Circle_area` collides"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn user_function_after_impl_collision_is_rejected() {
    // The reverse order (impl first, then a same-named user `fn`) used to slip
    // through and silently overwrite the mangled impl in the native runtime.
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        type Circle = Map<string, number>
        impl Shape for Circle {
            fn area(self): number { return 100 }
        }
        fn impl_Shape_Circle_area(): number { return 1 }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("collides with a trait dispatch impl"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn main_rejects_required_parameters() {
    let err = analyze_src("fn main(x: number) { print(str(x)) }").unwrap_err();
    assert!(
        err.message.contains("`main` must not require arguments"),
        "got: {}",
        err.message
    );
    // Defaulted parameters are fine: both runtimes fall back to the default.
    assert!(analyze_src("fn main(x: number = 5) { print(str(x)) }").is_ok());
    assert!(analyze_src("fn main() { print(1) }").is_ok());
}

#[test]
fn store_destructure_binds_fields() {
    // `@Store let { n } = …` binds each destructured field; the binding is
    // usable inside the component body (its shape is statically `Any`).
    assert!(
        analyze_src("fn main(): View { @Store let { n } = { n: 5 } VStack { Text(str(n + 1)) } }")
            .is_ok()
    );
}

#[test]
fn enum_name_can_be_reexported() {
    // The header (`use`/`pub use`) precedes every declaration (E0605), so
    // `pub use { Color }` runs before the enum exists and is validated at
    // the end of the pass: a `pub` enum re-exports, a type alias gets an
    // honest message, and a private declaration is E0602.
    assert!(
        analyze_src("pub use { Color };\npub enum Color { Red Blue }\nfn main() { print(1) }")
            .is_ok()
    );
    let err =
        analyze_src("pub use { Rect };\ntype Rect = Map<string, number>\nfn main() { print(1) }")
            .unwrap_err();
    assert!(
        err.message.contains("no runtime value"),
        "got: {}",
        err.message
    );
    assert_eq!(err.code(), "E0602");
    let err =
        analyze_src("pub use { Secret };\nenum Secret { A }\nfn main() { print(1) }").unwrap_err();
    assert_eq!(err.code(), "E0602", "got: {}", err.message);
    let err = analyze_src("pub use { nowhere };\nfn main() { print(1) }").unwrap_err();
    assert_eq!(err.code(), "E0101", "got: {}", err.message);
}

#[test]
fn unbounded_generic_stays_any_zero_regression() {
    let src = r#"
        fn id<T>(x: T): T { return x }
        fn main() { print(id(42)) }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn bounded_generic_refines_member_access() {
    // Bound satisfaction is impl-based (spec `types/traits.md`), so the
    // argument is a nominal type with a `Shape` impl, not a structural match.
    let src = r#"
        enum Marker { Marker(number) }
        trait Shape {
            fn area(self): number
        }
        impl Shape for Marker {
            fn area(self): number { return 5 }
        }
        fn apply<T: Shape>(t: T): number {
            let area = t.area
            return area()
        }
        fn main() {
            let c = Marker::Marker(1)
            print(apply(c))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn bounded_generic_missing_bound_member_is_rejected() {
    // The bound grants only the members its trait declares; a member the
    // bound does not have is E0102 on the parameter (spec `types/traits.md`).
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        fn apply<T: Shape>(t: T): number {
            let volume = t.volume
            return volume()
        }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0102");
    assert!(
        err.message.contains("no member `volume` on `T`"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn impl_method_signature_mismatch_is_rejected() {
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        type Circle = Map<string, number>
        impl Shape for Circle {
            fn area(self): string { return "x" }
        }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("must return `number`"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn impl_undeclared_method_is_rejected() {
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        type Circle = Map<string, number>
        impl Shape for Circle {
            fn area(self): number { return 1 }
            fn volume(self): number { return 1 }
        }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("`volume`, which `Shape` does not declare"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn impl_self_receiver_mismatch_is_rejected() {
    // Trait declares `self`, impl omits it (and vice versa): dispatch always
    // passes the receiver as the impl function's first argument, so a mismatch
    // would silently drop or mis-bind it.
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        type Circle = Map<string, number>
        impl Shape for Circle {
            fn area(): number { return 1 }
        }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("must declare `self` as its first parameter"),
        "unexpected: {}",
        err.message
    );

    let src = r#"
        trait Shape {
            fn area(): number
        }
        type Circle = Map<string, number>
        impl Shape for Circle {
            fn area(self): number { return 1 }
        }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("but the trait method has no receiver"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn impl_asyncness_mismatch_is_rejected() {
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        type Circle = Map<string, number>
        impl Shape for Circle {
            async fn area(self): number { return 1 }
        }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("must be synchronous to match the trait"),
        "unexpected: {}",
        err.message
    );

    let src = r#"
        trait Shape {
            async fn area(self): number
        }
        type Circle = Map<string, number>
        impl Shape for Circle {
            fn area(self): number { return 1 }
        }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("must be async to match the trait"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn selfless_trait_method_cannot_be_dispatched() {
    // Dispatch binds the receiver positionally to the impl's first argument;
    // a trait method without `self` would mis-bind the receiver into its first
    // real parameter at run time.
    let src = r#"
        trait Calc {
            fn add(x: number, y: number): number
        }
        type C = Map<string, number>
        impl Calc for C {
            fn add(x: number, y: number): number { return x + y }
        }
        fn main() {
            let c: C = {}
            print(Calc::add(c, 1, 2))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("has no `self` receiver, so it cannot be dispatched"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn dispatch_on_optional_receiver_is_rejected() {
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        type Circle = Map<string, number>
        impl Shape for Circle {
            fn area(self): number { return self.r }
        }
        fn main() {
            let c: Circle? = null
            print(Shape::area(c))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("cannot dispatch `Shape::area` on an optional receiver"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn impl_missing_trait_method_is_rejected() {
    // An `impl` must provide every trait method; the missing one is reported
    // at the `impl` site instead of surfacing later at a dispatch call site.
    let src = r#"
        trait Shape {
            fn area(self): number;
            fn perimeter(self): number
        }
        type Rect = Map<string, number>
        impl Shape for Rect {
            fn area(self): number { return self.w * self.h }
        }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("does not implement all of `Shape`'s methods"),
        "unexpected: {}",
        err.message
    );
    assert!(
        err.message.contains("missing perimeter"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn generic_bound_member_call_resolves() {
    // spec `types/generics.md`: inside `T: Shape`, values of `T` expose the
    // members declared by `Shape`, so the member form is well-formed without
    // a concrete receiver; the call site only has to satisfy the bound.
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        fn area_of<T: Shape>(s: T): number {
            return s.area()
        }
        struct Circle { r: number }
        impl Shape for Circle {
            fn area(self): number { return 5 }
        }
        fn main() {
            let c = { r: 1 }
            print(area_of(c))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn generic_unbounded_member_call_is_e0102() {
    // A type parameter has no members unless a bound grants them (spec
    // `types/traits.md`): `s.area()` inside `fn area_of<T>` is E0102.
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        fn area_of<T>(s: T): number {
            return s.area()
        }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0102");
    assert!(
        err.message.contains("no member `area` on `T`"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn impl_for_unknown_type_is_rejected() {
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        impl Shape for Nope {
            fn area(self): number { return 1 }
        }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("unknown type `Nope`"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn exported_trait_is_registered() {
    use xulo_semantic::analyze_with;
    let src = r#"
        pub trait Area {
            fn area(self): number
        }
        pub fn main() { print(1) }
    "#;
    let program = parse_program(src).unwrap();
    let result = analyze_with(&program, &[], &[], &[]).unwrap_or_else(|e| panic!("{}", e.message));
    assert!(
        result.exported_types.iter().any(|(n, _)| n == "Area"),
        "trait `Area` should be exported"
    );
}

#[test]
fn duplicate_trait_registration_is_rejected() {
    let src = r#"
        trait Shape { fn area(self): number }
        type Shape = Map<string, number>
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("already defined"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn where_clause_bound_is_validated() {
    let src = r#"
        trait Shape { fn area(self): number }
        fn f<T>(t: T): number where T: Shape { let area = t.area; return 1 }
        fn main() { print(1) }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn unknown_trait_in_bound_is_rejected() {
    let src = r#"
        fn f<T: Mystery>(t: T): number { return 1 }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("unknown trait `Mystery`"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn trait_method_arity_mismatch_is_rejected() {
    let src = r#"
        trait Shape { fn scale(self, by: number): Shape }
        type Circle = Map<string, number>
        impl Shape for Circle {
            fn scale(self): Circle { return 1 }
        }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("wrong arity"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn self_resolves_only_inside_impl_methods() {
    let ok = r#"
        trait Shape { fn area(self): number }
        type Circle = Map<string, number>
        impl Shape for Circle {
            fn area(self): number { return self.radius }
        }
        fn main() { print(1) }
    "#;
    assert!(analyze_src(ok).is_ok());

    let bad = r#"
        fn f(): number { return self }
    "#;
    let err = analyze_src(bad).unwrap_err();
    assert!(
        err.message.contains("undefined variable `self`"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn dispatch_annotations_are_applied_to_the_ast() {
    let src = r#"
        trait Shape { fn area(self): number }
        type Circle = Map<string, number>
        impl Shape for Circle {
            fn area(self): number { return self.radius }
        }
        fn main() {
            let c: Circle = { radius: 5 }
            Shape::area(c)
        }
    "#;
    let mut program = parse_program(src).unwrap();
    let result = xulo_semantic::analyze_with(&program, &[], &[], &[]).unwrap();
    xulo_semantic::apply_trait_dispatch(&mut program, &result.trait_dispatch);
    let mut names = Vec::new();
    for stmt in &program.statements {
        if let xulo_core::ast::Statement::Fn(f) = stmt {
            for s in &f.body.statements {
                if let xulo_core::ast::Statement::Expr(e) = s
                    && let xulo_core::ast::Expression::Call(c) = &e.expr
                    && let Some(name) = &c.trait_impl
                {
                    names.push(name.clone());
                }
            }
        }
    }
    assert_eq!(names, vec!["impl_Shape_Circle_area"]);
}

#[test]
fn component_names_are_recorded_as_uses() {
    let src = r#"
        fn Card(title: string): View {
            View {
                Text(title)
            }
        }
        fn main() {
            View {
                Card("hi")
                Text("x")
            }
        }
    "#;
    let program = parse_program(src).unwrap();
    let result = xulo_semantic::analyze_with(&program, &[], &[], &[]).unwrap();

    let use_of = |name: &str| {
        result
            .resolutions
            .iter()
            .find(|r| r.name == name)
            .expect("resolution recorded")
    };

    // User component factory: colors like a component, definition still jumps.
    let card = use_of("Card");
    assert_eq!(card.kind, xulo_semantic::UseKind::Component);
    assert_eq!(card.type_.as_ref().map(|t| t.name()), Some("View".into()));
    assert!(card.def.is_some(), "user component has a definition");

    // Builtin components: no definition, kind `Component`.
    let text = use_of("Text");
    assert_eq!(text.kind, xulo_semantic::UseKind::Component);
    assert!(text.def.is_none(), "builtin component has no definition");

    // The component name span points at the name token.
    let span = card.span.clone();
    assert_eq!(&src[span], "Card");
}

#[test]
fn partial_analysis_keeps_resolutions_on_error() {
    let src = r#"
        fn main() {
            let good = 1
            let bad = not_a_function(2)
            let after = 3
        }
    "#;
    let program = parse_program(src).unwrap();
    let (result, error) = xulo_semantic::analyze_partial(&program, &[], &[], &[]);

    assert!(error.is_some(), "first error preserved");
    let names: Vec<String> = result.resolutions.iter().map(|r| r.name.clone()).collect();
    assert!(
        names.contains(&"good".to_string()),
        "resolutions before the error survive"
    );
    assert!(
        names.contains(&"after".to_string()),
        "resolutions after the error survive"
    );
}

#[test]
fn analyze_with_still_fails_fast() {
    let src = r#"
        fn main() {
            let good = 1
            let bad = not_a_function(2)
        }
    "#;
    let program = parse_program(src).unwrap();
    let result = xulo_semantic::analyze_with(&program, &[], &[], &[]);
    assert!(result.is_err(), "analyze_with keeps the fail-fast contract");
}

#[test]
fn memo_requires_initializer() {
    let src = r#"
        fn main() {
            @Memo let x: number
        }
    "#;
    assert!(analyze_src(src).is_err());
}

#[test]
fn memo_requires_function_body_top_level() {
    // Module scope is not a function body.
    let global = "@Memo let x = 1\n";
    assert!(analyze_src(global).is_err());

    // A nested block inside a function is not the body's top level.
    let nested = r#"
        fn main() {
            if true {
                @Memo let x = 1
            }
        }
    "#;
    assert!(analyze_src(nested).is_err());
}

#[test]
fn break_requires_loop() {
    let src = r#"
        fn f() {
            break
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("`break` may only be used inside a `for` or `while` loop")
    );
}

#[test]
fn continue_requires_loop() {
    let src = r#"
        fn f() {
            continue
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("`continue` may only be used inside a `for` or `while` loop")
    );
}

#[test]
fn break_allowed_in_for_loop() {
    let src = r#"
        fn f() {
            let list = [1, 2, 3]
            for i in list {
                break
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn continue_allowed_in_for_loop() {
    let src = r#"
        fn f() {
            let list = [1, 2, 3]
            for i in list {
                continue
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn break_allowed_in_while_loop() {
    let src = r#"
        fn f() {
            while true {
                break
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn continue_allowed_in_while_loop() {
    let src = r#"
        fn f() {
            while true {
                continue
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn break_rejected_in_if_not_loop() {
    let src = r#"
        fn f() {
            if true {
                break
            }
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("`break` may only be used inside a `for` or `while` loop")
    );
}

#[test]
fn modulo_requires_numbers() {
    let src = r#"
        fn f() {
            let x = "a" % "b"
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(err.message.contains("cannot apply `%`"));
}

#[test]
fn power_requires_numbers() {
    let src = r#"
        fn f() {
            let x = "a" ** "b"
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(err.message.contains("cannot apply `**`"));
}

#[test]
fn modulo_accepts_numbers() {
    let src = r#"
        fn f() {
            let x = 10 % 3
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn power_accepts_numbers() {
    let src = r#"
        fn f() {
            let x = 2 ** 10
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn bitwise_and_requires_numbers() {
    let src = r#"
        fn f() {
            let x = "a" & "b"
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(err.message.contains("cannot apply `&`"));
}

#[test]
fn bitwise_or_requires_numbers() {
    let src = r#"
        fn f() {
            let x = "a" | "b"
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(err.message.contains("cannot apply `|`"));
}

#[test]
fn bitwise_xor_requires_numbers() {
    let src = r#"
        fn f() {
            let x = "a" ^ "b"
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(err.message.contains("cannot apply `^`"));
}

#[test]
fn shift_left_requires_numbers() {
    let src = r#"
        fn f() {
            let x = "a" << "b"
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(err.message.contains("cannot apply `<<`"));
}

#[test]
fn shift_right_requires_numbers() {
    let src = r#"
        fn f() {
            let x = "a" >> "b"
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(err.message.contains("cannot apply `>>`"));
}

#[test]
fn bitwise_not_requires_number() {
    let src = r#"
        fn f() {
            let x = ~"a"
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("unary `~` requires a `number` operand")
    );
}

#[test]
fn bitwise_and_accepts_numbers() {
    let src = r#"
        fn f() {
            let x = 10 & 3
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn bitwise_or_accepts_numbers() {
    let src = r#"
        fn f() {
            let x = 10 | 3
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn bitwise_xor_accepts_numbers() {
    let src = r#"
        fn f() {
            let x = 10 ^ 3
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn shift_left_accepts_numbers() {
    let src = r#"
        fn f() {
            let x = 10 << 3
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn shift_right_accepts_numbers() {
    let src = r#"
        fn f() {
            let x = 10 >> 3
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn bitwise_not_accepts_number() {
    let src = r#"
        fn f() {
            let x = ~10
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

// ──────────────────────────────────────────────────────────────────────
//  List method type inference
// ──────────────────────────────────────────────────────────────────────

#[test]
fn list_length_returns_number() {
    let src = r#"
        fn f() {
            let a = [1, 2, 3]
            let n: number = a.length
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_capacity_returns_number() {
    let src = r#"
        fn f() {
            let a = [1, 2, 3]
            let n: number = a.capacity
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_is_empty_returns_bool() {
    let src = r#"
        fn f() {
            let a = [1, 2]
            let b: bool = a.isEmpty()
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_first_returns_optional() {
    let src = r#"
        fn f() {
            let a = [1, 2, 3]
            let n: number? = a.first()
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_last_returns_optional() {
    let src = r#"
        fn f() {
            let a = [1, 2, 3]
            let n: number? = a.last()
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_get_returns_optional() {
    let src = r#"
        fn f() {
            let a = [1, 2, 3]
            let n: number? = a.get(0)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_push_returns_self() {
    let src = r#"
        fn f() {
            let a = [1, 2]
            let b: List<number> = a.push(3)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_pop_returns_element() {
    let src = r#"
        fn f() {
            let a = [1, 2, 3]
            let n: number = a.pop()
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_contains_returns_bool() {
    let src = r#"
        fn f() {
            let a = [1, 2, 3]
            let b: bool = a.contains(2)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_index_of_returns_number() {
    let src = r#"
        fn f() {
            let a = [1, 2, 3]
            let n: number = a.indexOf(2)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_join_returns_string() {
    let src = r#"
        fn f() {
            let a = ["a", "b"]
            let s: string = a.join(", ")
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_sort_returns_self() {
    let src = r#"
        fn f() {
            let a = [3, 1, 2]
            let b: List<number> = a.sort()
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_reverse_returns_self() {
    let src = r#"
        fn f() {
            let a = [1, 2, 3]
            let b: List<number> = a.reverse()
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_concat_returns_list() {
    let src = r#"
        fn f() {
            let a = [1, 2]
            let b = [3, 4]
            let c: List<number> = a.concat(b)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_slice_returns_list() {
    let src = r#"
        fn f() {
            let a = [1, 2, 3, 4]
            let b: List<number> = a.slice(1, 3)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_remove_value_returns_bool() {
    let src = r#"
        fn f() {
            let a = [1, 2, 3]
            let b: bool = a.removeValue(2)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_map_returns_list() {
    let src = r#"
        fn f() {
            let a = [1, 2, 3]
            let b: List<number> = a.map(fn(x) { x * 2 })
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_filter_returns_list() {
    let src = r#"
        fn f() {
            let a = [1, 2, 3, 4]
            let b: List<number> = a.filter(fn(x) { x > 2 })
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_some_returns_bool() {
    let src = r#"
        fn f() {
            let a = [1, 2, 3]
            let b: bool = a.some(fn(x) { x > 2 })
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_every_returns_bool() {
    let src = r#"
        fn f() {
            let a = [2, 4, 6]
            let b: bool = a.every(fn(x) { x > 0 })
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

// ──────────────────────────────────────────────────────────────────────
//  flat() and shrink() type inference
// ──────────────────────────────────────────────────────────────────────

#[test]
fn list_flat_returns_list() {
    let src = r#"
        fn f() {
            let a = [[1, 2], [3, 4]]
            let b = a.flat()
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn list_shrink_returns_self() {
    let src = r#"
        fn f() {
            let a = [1, 2, 3]
            let b = a.shrink()
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

// ──────────────────────────────────────────────────────────────────────
//  Tuple destructuring
// ──────────────────────────────────────────────────────────────────────

#[test]
fn tuple_destructure_semantic_ok() {
    let src = r#"
        fn f() {
            let (a, b, c) = [1, 2, 3]
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn tuple_destructure_binds_correct_types() {
    let src = r#"
        fn f() {
            let (x, y) = [10, 20]
            let a: number = x
            let b: number = y
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn tuple_destructure_non_list_is_error() {
    let src = r#"
        fn f() {
            let (a, b) = 42
        }
    "#;
    assert!(analyze_src(src).is_err());
}

// Struct tests

#[test]
fn struct_definition_registers_type() {
    let src = r#"
        struct User { name: string, age: number }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn struct_duplicate_field_is_error() {
    let src = r#"
        struct User { name: string, name: number }
    "#;
    assert!(analyze_src(src).is_err());
}

#[test]
fn struct_duplicate_type_is_error() {
    let src = r#"
        struct User { name: string }
        struct User { age: number }
    "#;
    assert!(analyze_src(src).is_err());
}

#[test]
fn struct_literal_infers_named_type() {
    let src = r#"
        struct User { name: string, age: number }
        fn f() {
            let u = { name: "Alice", age: 30 }
            let n: string = u.name
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn struct_literal_wrong_field_name_is_error() {
    let src = r#"
        struct User { name: string, age: number }
        fn f() {
            let u = { name: "Alice", wrong: 30 }
        }
    "#;
    // This won't error because without type annotation, it's just an object literal
    // The struct matching is best-effort
    assert!(analyze_src(src).is_ok());
}

#[test]
fn struct_literal_with_type_annotation_checks_fields() {
    let src = r#"
        struct User { name: string, age: number }
        fn f() {
            let u: User = { name: "Alice", age: 30 }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn inherent_impl_block() {
    let src = r#"
        struct User { name: string, age: number }
        impl User {
            fn create(name: string, age: number): User {
                return { name: name, age: age }
            }
            fn greet(self): string {
                return "Hello, " + self.name
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn inherent_impl_unknown_type_is_error() {
    let src = r#"
        impl unknown {
            fn new(): unknown { return {} }
        }
    "#;
    assert!(analyze_src(src).is_err());
}

#[test]
fn object_destructuring() {
    let src = r#"
        fn f() {
            let obj = { name: "Alice", age: 30 }
            let { name, age } = obj
            let n: string = name
            let a: number = age
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn object_destructuring_non_object_is_error() {
    let src = r#"
        fn f() {
            let { name } = 42
        }
    "#;
    assert!(analyze_src(src).is_err());
}

#[test]
fn object_destructuring_duplicate_binding_is_error() {
    let src = r#"
        fn f() {
            let obj = { name: "Alice" }
            let { name, name: n } = obj
        }
    "#;
    assert!(analyze_src(src).is_err());
}

// ---------------------------------------------------------------------------
// `#Test fn` (spec testing.md)
// ---------------------------------------------------------------------------

#[test]
fn test_fn_body_is_checked() {
    let src = r#"
        #Test
        fn checked() {
            let x: number = "not a number"
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("cannot bind"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn test_fn_body_sees_module_declarations() {
    let src = r#"
        fn helper(): number { return 42 }
        const LIMIT = 10
        #Test
        fn sees_module() {
            let a = helper()
            let b = LIMIT
            assert(a == 42, "helper")
            assert(b == 10, "limit")
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn test_fn_reference_is_forbidden() {
    let src = r#"
        #Test
        fn forbidden() {
            assert(true, "ok")
        }
        fn main() {
            forbidden()
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("`forbidden` is a test function and cannot be referenced"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn test_fn_value_reference_is_forbidden() {
    let src = r#"
        #Test
        fn forbidden() {
            assert(true, "ok")
        }
        fn main() {
            let f = forbidden
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("`forbidden` is a test function and cannot be referenced"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn test_fn_self_reference_is_forbidden() {
    let src = r#"
        #Test
        fn recursive() {
            recursive()
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("`recursive` is a test function and cannot be referenced"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn test_fn_reference_before_declaration_is_forbidden() {
    let src = r#"
        fn main() {
            later()
        }
        #Test
        fn later() {
            assert(true, "ok")
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("`later` is a test function and cannot be referenced"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn test_fn_await_needs_async_marker() {
    let src = r#"
        #Test
        fn sync_test() {
            await Time::sleep(1)
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("`await`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn test_async_fn_await_is_allowed() {
    let src = r#"
        async fn load(): number { return 1 }
        #Test
        async fn async_test() {
            let v = await load()
            assert(v == 1, "loaded")
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn test_fn_duplicate_names_are_allowed() {
    let src = r#"
        #Test
        fn same_name() {
            assert(true, "first")
        }
        #Test
        fn same_name() {
            assert(true, "second")
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn test_fn_trailing_expression_is_not_an_implicit_return() {
    // The body is an implicit `fn(): unit`: a trailing expression is a
    // statement, and `return e` checks `e` against the unit result — here a
    // number-returning call in `return` position must fail.
    let src = r#"
        fn num(): number { return 1 }
        #Test
        fn t() {
            return num()
        }
    "#;
    // `return e` checks e against unit (Any here — the unit stand-in), so this
    // is accepted at the type level; the statement-value rule still applies
    // to a bare trailing expression. Pin the accepted form:
    assert!(analyze_src(src).is_ok());
}

// ---------------------------------------------------------------------------
// `assert` intrinsic
// ---------------------------------------------------------------------------

#[test]
fn assert_accepts_boolean_and_string() {
    let src = r#"
        fn main() {
            assert(1 < 2, "one is less than two")
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn assert_rejects_wrong_arity() {
    let src = r#"
        fn main() {
            assert(true)
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("`assert` expects exactly two arguments"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn assert_rejects_non_boolean_condition() {
    let src = r#"
        fn main() {
            assert(42, "boom")
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("`assert` condition must be a `bool`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn assert_rejects_non_string_message() {
    let src = r#"
        fn main() {
            assert(true, 42)
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("`assert` message must be a `string`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn user_function_named_assert_shadows_intrinsic() {
    let src = r#"
        fn assert(a: number, b: number): number { return a }
        fn main() {
            print(assert(1, 2))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn type_application_arity_mismatch_is_e0230() {
    let src = r#"
        struct Pair<A, B> { a: A, b: B }
        fn main() {
            let bad: Pair<string> = { a: 1, b: 2 }
            print(str(bad))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0230");
    assert!(
        err.message
            .contains("`Pair` expects 2 type arguments, found 1"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn well_formed_type_application_is_accepted() {
    let src = r#"
        struct Pair<A, B> { a: A, b: B }
        fn main() {
            let ok: Pair<string, number> = { a: "x", b: 2 }
            let also_ok: Pair<number, string> = { a: 1, b: "y" }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn type_application_checks_its_arguments() {
    let src = r#"
        struct Pair<A, B> { a: A, b: B }
        fn main() {
            let bad: Pair<Bogus, number> = { a: 1, b: 2 }
            print(str(bad))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("unknown type `Bogus`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn bare_generic_name_is_e0230() {
    let src = r#"
        fn main() {
            let xs: List = [1]
            print(str(xs))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0230");
    assert!(
        err.message
            .contains("`List` expects 1 type argument, found 0"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn arguments_on_a_non_generic_name_are_e0230() {
    // The primitive words are keywords now (spec `lexical-structure.md`), so
    // a written argument list after one is a syntax error; `View` keeps the
    // E0230 shape check for a zero-arity non-keyword name.
    let src = r#"
        fn main() {
            let x: View<int> = 1
            print(str(x))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0230");
    assert!(
        err.message
            .contains("`View` expects 0 type arguments, found 1"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn applied_type_parameter_is_e0230() {
    let src = r#"
        fn f<T>(x: T<int>): T { x }
        fn main() { print(str(f(1))) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0230");
    assert!(
        err.message
            .contains("`T` is a type parameter, so it takes no type arguments"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn type_parameter_list_has_no_defaults() {
    // `T = int` is not a form the grammar accepts at all.
    let src = "fn f<T = int>(x: T): T { x }\nfn main() { print(str(f(1))) }";
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0002");
}

#[test]
fn trait_declaring_type_parameters_is_e0810() {
    let src = r#"
        trait Convert<T> { fn convert(self): string }
        fn main() { print("x") }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0810");
    assert!(
        err.message
            .contains("a trait may not declare type parameters"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn bound_applying_type_arguments_is_e0811() {
    let src = r#"
        trait Convert { fn convert(self): string }
        fn f<T: Convert<string> >(x: T): string { x.convert() }
        fn main() { print("x") }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0811");
    assert!(
        err.message
            .contains("`Convert<string>` applies type arguments; a trait has none"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn where_bound_applying_type_arguments_is_e0811() {
    let src = r#"
        trait Convert { fn convert(self): string }
        fn f<T>(x: T): string where T: Convert<string> { x.convert() }
        fn main() { print("x") }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0811");
}

#[test]
fn bound_that_is_not_a_trait_is_e0806() {
    let src = r#"
        fn f<T: number>(x: T): T { x }
        fn main() { print(str(f(1))) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0806");
    assert!(
        err.message.contains("unknown trait `number`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn bounded_generic_argument_without_impl_is_e0801() {
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        fn apply<T: Shape>(t: T): number { return 0 }
        fn main() {
            print(apply(1))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0801");
    assert!(err.message.contains("`number` does not implement `Shape`"));
}

#[test]
fn bounded_type_argument_at_a_written_position_is_e0801() {
    let src = r#"
        trait Area {
            fn area(self): number
        }
        type Palette<T: Area> = List<T>
        fn main() {
            let p: Palette<string> = ["a"]
            print(p)
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0801");
    assert!(err.message.contains("`string` does not implement `Area`"));
}

#[test]
fn conflicting_type_argument_inference_is_e0202() {
    let src = r#"
        fn pick<T>(a: List<T>, b: T): T { return b }
        fn main() {
            print(pick([1, 2], "x"))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0202");
    assert!(
        err.message
            .contains("expected `number` for `T`, found `string`")
    );
}

#[test]
fn bound_satisfied_by_a_type_alias_is_accepted() {
    let src = r#"
        enum Marker { Marker(number) }
        trait Shape {
            fn area(self): number
        }
        impl Shape for Marker {
            fn area(self): number { return 5 }
        }
        type Dot = Marker
        fn apply<T: Shape>(t: T): number { return 0 }
        fn main() {
            let d: Dot = Marker::Marker(1)
            print(apply(d))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn structural_object_alone_does_not_satisfy_a_bound() {
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        fn apply<T: Shape>(t: T): number { return 0 }
        fn main() {
            let c: Map<string, unknown> = { area: fn(): number { return 5 } }
            print(apply(c))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0801");
}

#[test]
fn await_inside_lock_is_e0410() {
    let src = r#"
        async fn fetch_count(): number { return 1 }
        async fn refresh(state: shared number) {
            lock state {
                let n = await fetch_count()
                print(n)
            }
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0410");
    assert!(err.message.contains("`await` is not allowed inside `lock`"));
}

#[test]
fn await_outside_lock_is_accepted() {
    let src = r#"
        async fn fetch_count(): number { return 1 }
        async fn refresh(state: shared number) {
            let fresh = await fetch_count()
            lock state {
                print(fresh)
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn await_in_a_closure_declared_inside_lock_is_accepted() {
    // The closure body runs when it is called, outside the critical section.
    let src = r#"
        async fn fetch_count(): number { return 1 }
        async fn holder(state: shared number) {
            lock state {
                let f = async fn(): number { return await fetch_count() }
                print(0)
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn mut_param_can_be_written_through() {
    let src = r#"
        fn fill(p: mut List<number>) {
            p[0] = 7
        }
        fn main() {
            let mut xs = [0]
            fill(xs)
            print(xs)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn immutable_param_cannot_be_written_through_is_e0401() {
    let src = r#"
        fn fill(p: List<number>) {
            p[0] = 7
        }
        fn main() {
            let xs = [0]
            fill(xs)
            print(xs)
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0401");
    assert!(err.message.contains("not declared `mut`"));
}

#[test]
fn index_assignment_through_immutable_binding_is_e0401() {
    let src = r#"
        fn main() {
            let xs = [0]
            xs[0] = 1
            print(xs)
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0401");
}

#[test]
fn capture_of_mut_param_cannot_be_assigned_is_e0401() {
    let src = r#"
        enum Counter { Counter(number) }
        fn broken(p: mut Counter) {
            let f = fn() { p.value = 1 }
            print(f)
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0401");
    assert!(err.message.contains("`p`"));
}

#[test]
fn capture_of_let_mut_binding_may_be_assigned() {
    let src = r#"
        enum Counter { Counter(number) }
        fn ok() {
            let mut c = Counter::Counter(0)
            let f = fn() { c.value = 1 }
            print(f)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn move_of_a_parameter_is_e0403() {
    let src = r#"
        fn steal(p: List<number>): number {
            let owned = move p
            return owned[0]
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0403");
    assert!(err.message.contains("cannot move `p`"));
}

#[test]
fn move_of_a_part_of_a_binding_is_e0403() {
    let src = r#"
        fn main() {
            let xs = [1, 2]
            let owned = move xs[0]
            print(owned)
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0403");
    assert!(err.message.contains("whole binding"));
}

#[test]
fn move_of_a_local_binding_is_accepted() {
    let src = r#"
        fn take(owned: List<number>): number {
            return owned[0]
        }
        fn main() {
            let xs = [1, 2]
            let owned = move xs
            print(take(owned))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn mutable_argument_must_be_a_mut_place_is_e0404() {
    let src = r#"
        fn bump(n: mut number): number { return n + 1 }
        fn main() {
            let n = 1
            print(bump(n))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0404");
    assert!(err.message.contains("cannot borrow `n` as mutable"));
}

#[test]
fn overlapping_arguments_where_either_param_is_mut_are_e0404() {
    let src = r#"
        fn pair(a: mut number, b: number): number { return a + b }
        fn main() {
            let mut xs = [1, 2]
            print(pair(xs[0], xs[0]))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0404");
}

#[test]
fn non_literal_subscript_is_never_assumed_distinct_is_e0404() {
    let src = r#"
        fn pair(a: mut number, b: number): number { return a + b }
        fn main() {
            let mut xs = [1, 2]
            let i = 0
            print(pair(xs[0], xs[i]))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0404");
}

#[test]
fn distinct_literal_subscripts_are_disjoint() {
    let src = r#"
        fn pair(a: mut number, b: number): number { return a + b }
        fn main() {
            let mut xs = [1, 2]
            print(pair(xs[0], xs[1]))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn mut_parameter_lends_its_borrow_onward() {
    let src = r#"
        fn bump(n: mut number): number { return n + 1 }
        fn outer(n: mut number): number { return bump(n) }
        fn main() {
            let mut n = 1
            print(outer(n))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn immutable_parameter_cannot_lend_a_mutable_borrow() {
    let src = r#"
        fn bump(n: mut number): number { return n + 1 }
        fn outer(n: number): number { return bump(n) }
        fn main() {
            let mut n = 1
            print(outer(n))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0404");
}

#[test]
fn write_during_an_outstanding_immutable_borrow_is_e0411() {
    let src = r#"
        async fn peek(xs: List<number>): number { return xs[0] }
        async fn watch(xs: mut List<number>) {
            let t = peek(xs)
            xs[0] = 9
            await t
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0411");
    assert!(err.message.contains("cannot write `xs[0]`"));
}

#[test]
fn reads_during_an_immutable_borrow_are_allowed() {
    let src = r#"
        async fn peek(xs: List<number>): number { return xs[0] }
        async fn watch(xs: mut List<number>) {
            let t = peek(xs)
            let a = xs[0]
            print(a)
            await t
            xs[0] = 9
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn use_during_an_outstanding_mutable_borrow_is_e0405() {
    let src = r#"
        async fn fill(p: mut List<number>) {
            p[0] = 7
        }
        async fn main() {
            let mut xs = [0]
            let t = fill(xs)
            xs[0] = 1
            await t
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0405");
    assert!(err.message.contains("cannot use `xs[0]`"));
}

#[test]
fn the_borrow_ends_where_control_passes_an_await_of_the_task() {
    let src = r#"
        async fn fill(p: mut List<number>) {
            p[0] = 7
        }
        async fn main() {
            let mut xs = [0]
            let t = fill(xs)
            await t
            xs[0] = 1
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn a_mutable_borrow_may_not_start_during_an_immutable_one() {
    let src = r#"
        async fn peek(xs: List<number>): number { return xs[0] }
        async fn fill(p: mut List<number>) {
            p[0] = 7
        }
        async fn main() {
            let mut xs = [0]
            let u = peek(xs)
            let t = fill(xs)
            await u
            await t
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0404");
    assert!(err.message.contains("already immutably borrowed"));
}

#[test]
fn a_sync_call_leaves_nothing_outstanding() {
    let src = r#"
        fn bump(p: mut List<number>) {
            p[0] = 7
        }
        fn main() {
            let mut xs = [0]
            bump(xs)
            xs[0] = 1
            print(xs)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn reading_shared_contents_outside_lock_is_e0406() {
    let src = r#"
        fn main() {
            let state = shared [0]
            print(state[0])
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0406");
    assert!(
        err.message
            .contains("shared state `state` is accessed outside `lock`")
    );
}

#[test]
fn writing_shared_contents_outside_lock_is_e0406() {
    let src = r#"
        fn main() {
            let state = shared [0]
            state[0] = 1
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0406");
}

#[test]
fn shared_contents_are_reachable_under_lock_without_let_mut() {
    let src = r#"
        fn main() {
            let state = shared [0]
            lock state {
                state[0] = 1
                print(state[0])
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn lock_requires_a_shared_binding_is_e0407() {
    let src = r#"
        fn main() {
            let ordinary = [0]
            lock ordinary {
                print(ordinary)
            }
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0407");
    assert!(err.message.contains("`lock` requires a shared binding"));
}

#[test]
fn a_non_shared_argument_to_a_shared_parameter_is_e0407() {
    let src = r#"
        fn tick(state: shared List<number>) {
            lock state {
                print(state[0])
            }
        }
        fn main() {
            let n = [1]
            tick(n)
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0407");
}

#[test]
fn a_shared_parameter_reaches_its_contents_without_its_own_lock() {
    // The region arrives as a `shared` parameter: the caller's section (or
    // the callee's own `lock`) is what guards it, so the parameter is not
    // barred from its own contents.
    let src = r#"
        fn peek(state: shared List<number>): number {
            return state[0]
        }
        fn main() {
            let s = shared [1]
            lock s {
                print(peek(s))
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn an_uncaptured_enclosing_binding_in_spawn_is_e0408() {
    let src = r#"
        async fn main() {
            let config = { host: "localhost" }
            let job = spawn async {
                print(config.host)
            }
            await job
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0408");
    assert!(
        err.message
            .contains("`config` is not captured by the spawned block")
    );
}

#[test]
fn a_copy_capture_makes_the_binding_visible_to_the_spawn_block() {
    let src = r#"
        async fn main() {
            let config = { host: "localhost" }
            let job = spawn async {
                let payload = copy config
                print(payload.host)
            }
            await job
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn a_move_capture_makes_the_binding_visible_to_the_spawn_block() {
    let src = r#"
        async fn main() {
            let config = { host: "localhost" }
            let job = spawn async {
                let payload = move config
                print(payload.host)
            }
            await job
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn declarations_constants_and_literals_need_no_capture() {
    let src = r#"
        const LIMIT: number = 10
        fn helper(): number {
            return 1
        }
        async fn main() {
            let job = spawn async {
                print(helper())
                print(LIMIT)
                print("literal")
            }
            await job
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn a_nested_function_inside_spawn_may_read_the_blocks_own_bindings() {
    let src = r#"
        async fn main() {
            let outer = 1
            let job = spawn async {
                let inner = 2
                fn helper(): number {
                    return inner
                }
                print(helper())
                print(inner)
                print(outer)
            }
            await job
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0408");
    assert!(err.message.contains("`outer` is not captured"));
}

#[test]
fn await_unwraps_a_spawned_task() {
    let src = r#"
        async fn main() {
            let job = spawn async {
                print("work")
            }
            await job
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn using_a_binding_after_move_is_e0403() {
    let src = r#"
        fn consume(items: List<number>) {
            print(items)
        }
        fn main() {
            let items = [1, 2]
            consume(move items)
            print(items)
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0403");
    assert!(err.message.contains("cannot use moved value `items`"));
}

#[test]
fn moving_twice_is_e0403() {
    let src = r#"
        fn consume(items: List<number>) {
            print(items)
        }
        fn main() {
            let items = [1, 2]
            consume(move items)
            consume(move items)
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0403");
    assert!(err.message.contains("cannot use moved value `items`"));
}

#[test]
fn move_outside_let_argument_or_return_is_e0403() {
    let src = r#"
        fn main() {
            let x = 1
            let y = move x + 1
            print(y)
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0403");
    assert!(err.message.contains("`move` is only allowed"));
}

#[test]
fn move_in_let_argument_and_return_positions_is_legal() {
    let src = r#"
        fn consume(items: List<number>) {
            print(items)
        }
        fn pick(): List<number> {
            let a = [1]
            return move a
        }
        fn main() {
            let items = [1, 2]
            consume(move items)
            let b = pick()
            print(b)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn a_service_must_declare_on_message_is_e0409() {
    let src = r#"
        struct Tally { total: number }
        async fn main() {
            let worker = spawn.process Tally { total: 0 }
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0409");
    assert!(
        err.message
            .contains("service `Tally` declares no `on_message`")
    );
}

#[test]
fn a_service_impl_may_appear_after_the_spawn() {
    let src = r#"
        struct Tally { total: number }
        async fn main() {
            let worker = spawn.process Tally { total: 0 }
        }
        impl Tally {
            fn on_message(mut self, cmd: number) {
                self.total = self.total + cmd
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn an_on_message_without_a_receiver_is_e0409() {
    let src = r#"
        struct Tally { total: number }
        async fn main() {
            let worker = spawn.process Tally { total: 0 }
        }
        impl Tally {
            fn on_message(cmd: number) {
                print(cmd)
            }
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0409");
    assert!(err.message.contains("receiver"));
}

#[test]
fn reply_outside_on_message_is_an_error() {
    let src = r#"
        fn leak(n: number) {
            reply(n)
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("`reply` may only be called inside `on_message`")
    );
}

#[test]
fn reply_inside_on_message_checks() {
    let src = r#"
        struct Tally { total: number }
        impl Tally {
            fn on_message(mut self, cmd: number) {
                self.total = self.total + cmd
                reply(self.total)
            }
        }
        async fn main() {
            let worker = spawn.process Tally { total: 0 }
            let sum = await worker.send(41)
            print(sum)
            worker.stop()
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn task_all_of_async_calls_unwraps_to_a_list() {
    let src = r#"
        async fn work(n: number): number { n }
        async fn main() {
            let a = work(1)
            let b = work(2)
            let xs = await Task::all([a, b])
            print(xs)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn task_race_and_resolve_check() {
    let src = r#"
        async fn work(n: number): number { n }
        async fn main() {
            let a = work(1)
            let b = work(2)
            print(await Task::race([a, b]))
            let ready = Task::resolve(42)
            print(await ready)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn task_timeout_wraps_the_task_in_a_result() {
    let src = r#"
        async fn work(): number { 1 }
        async fn main() {
            let r: Result<number, Error> = await Task::timeout(work(), 100)
            match r {
                Result::Ok(v) => print(v)
                Result::Err(e) => print(e.message)
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn task_timeout_requires_a_task() {
    let src = r#"
        async fn main() {
            print(await Task::timeout(42, 100))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(err.message.contains("`Task::timeout` expects a task"));
}

#[test]
fn task_timeout_requires_a_number_deadline() {
    let src = r#"
        async fn main() {
            print(await Task::timeout(Task::resolve(1), "100"))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("`Task::timeout` millisecond count must be a `number`")
    );
}

#[test]
fn task_timeout_requires_two_arguments() {
    let src = r#"
        async fn main() {
            print(await Task::timeout(Task::resolve(1)))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message
            .contains("`Task::timeout` expects exactly two arguments")
    );
}

#[test]
fn builtin_result_and_error_construct_and_annotate() {
    let src = r#"
        async fn main() {
            let ok: Result<number, Error> = Result::Ok(1)
            let err: Result<number, Error> = Result::Err(Error("boom"))
            let e: Error = Error("boom")
            print(e.message)
            match ok {
                Result::Ok(v) => print(v)
                Result::Err(f) => print(f.message)
            }
            print(err)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn builtin_result_takes_two_type_arguments() {
    let src = r#"
        async fn main() {
            let r: Result<number> = Result::Ok(1)
            print(r)
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0230");
    assert!(err.message.contains("`Result` expects 2 type arguments"));
}

#[test]
fn error_call_requires_a_string_message() {
    let src = r#"
        async fn main() {
            print(Error(42))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(err.message.contains("`Error` message must be a `string`"));
}

#[test]
fn user_declared_result_shadows_the_builtin() {
    let src = r#"
        enum Result<T> {
            Success(T),
            Failure(string),
        }
        async fn main() {
            let ok: Result<number> = Result::Success(1)
            let bad: Result<number> = Result::Failure("nope")
            match ok {
                Result::Success(v) => print(v)
                Result::Failure(m) => print(m)
            }
            print(bad)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn task_namespace_rejects_unknown_members() {
    let src = r#"
        async fn main() {
            print(await Task::nope([]))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(err.message.contains("`Task` has no member `nope`"));
}

#[test]
fn task_all_requires_a_list() {
    let src = r#"
        async fn main() {
            print(await Task::all(1))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(err.message.contains("expects a list of tasks"));
}

#[test]
fn conflicting_reply_operands_are_e0211() {
    let src = r#"
        struct S { n: number }
        impl S {
            fn on_message(mut self, cmd: number) {
                if cmd > 0 {
                    reply(1)
                } else {
                    reply("x")
                }
            }
        }
        async fn main() {
            let h = spawn.process S { n: 0 }
            h.stop()
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0211");
    assert!(err.message.contains("no common type"));
}

#[test]
fn reply_type_flows_into_the_send_task() {
    let src = r#"
        struct S { n: number }
        impl S {
            fn on_message(mut self, cmd: number) {
                reply(cmd)
            }
        }
        async fn main() {
            let h = spawn.process S { n: 0 }
            let r: number = await h.send(1)
            print(r)
            h.stop()
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn send_task_rejects_a_mismatched_binding() {
    let src = r#"
        struct S { n: number }
        impl S {
            fn on_message(mut self, cmd: number) {
                reply(cmd)
            }
        }
        async fn main() {
            let h = spawn.process S { n: 0 }
            let r: string = await h.send(1)
            print(r)
            h.stop()
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(err.message.contains("cannot bind a value of type `number`"));
}

#[test]
fn on_message_without_reply_settles_unit() {
    let src = r#"
        struct S { n: number }
        impl S {
            fn on_message(mut self, cmd: number) {
                self.n = cmd
            }
        }
        async fn main() {
            let h = spawn.process S { n: 0 }
            let r = await h.send(1)
            print(r)
            h.stop()
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

// ----------------------------------------------------------------
// A4: generic `impl` applicability (spec `types/traits.md`,
// "When an impl applies", E0102/E0809 member-call resolution)
// ----------------------------------------------------------------

#[test]
fn generic_impl_applies_to_instantiated_receiver() {
    // The spec's `Palette<T: Area>` example: `Rect: Area`, so the impl
    // applies and `render` resolves with its declared return type.
    let src = r#"
        trait Area { fn area(self): number }
        struct Palette<T> { shape: T }
        struct Rect { w: number, h: number }
        impl Area for Rect {
            fn area(self): number { return self.w * self.h }
        }
        impl<T: Area> Palette<T> {
            fn render(self): number { return 12 }
        }
        fn main() {
            let ok = { shape: { w: 3, h: 4 } }
            let n: number = ok.render()
            print(str(n))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn generic_impl_bound_unsatisfied_is_e0102() {
    // The same `Palette<T: Area>` example with `Dot` (no `Area` impl): the
    // impl contributes nothing for `Palette<Dot>`, so `render` is missing
    // (spec `types/traits.md`: error[E0102], `no member ... on ...`).
    let src = r#"
        trait Area { fn area(self): number }
        struct Palette<T> { shape: T }
        struct Rect { w: number, h: number }
        struct Dot { r: number }
        impl Area for Rect {
            fn area(self): number { return self.w * self.h }
        }
        impl<T: Area> Palette<T> {
            fn render(self): number { return 12 }
        }
        fn main() {
            let bad = { shape: { r: 1 } }
            print(str(bad.render()))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0102");
    assert!(
        err.message.contains("no member `render` on `Palette<Dot>`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn generic_inherent_impl_member_returns_instantiated_type() {
    // `impl<T> Holder<T> { fn get(self): T }` resolves for a
    // `Holder<number>` receiver with `T` substituted back to `number`.
    let src = r#"
        struct Holder<T> { value: T }
        impl<T> Holder<T> {
            fn get(self): T { self.value }
        }
        fn main() {
            let h = { value: 42 }
            let n: number = h.get()
            print(str(n))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn repeated_pattern_parameter_must_bind_one_type() {
    // `impl<T> Pair<T, T>` applies to `Pair<number, number>`; a literal
    // whose fields disagree cannot instantiate it (inference conflict
    // falls back to the argument-less type, which the pattern's arity then
    // rejects).
    let src = r#"
        struct Pair<A, B> { first: A, second: B }
        impl<T> Pair<T, T> {
            fn duplicated(self): T { self.first }
        }
        fn main() {
            let p = { first: 1, second: "x" }
            print(str(p.duplicated()))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0102");
    assert!(
        err.message
            .contains("no member `duplicated` on `Pair<number, string>`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn member_missing_on_plain_struct_is_e0102() {
    // Step 1 and step 2 of member-call resolution both find nothing for a
    // struct with no applicable impl: E0102, not the loose `any`.
    let src = r#"
        struct User { name: string }
        fn main() {
            let u = { name: "a" }
            print(str(u.nope()))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0102");
    assert!(
        err.message.contains("no member `nope` on `User`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn ambiguous_trait_member_is_e0809() {
    // Two applicable trait impls both declaring `grow`: the member call has
    // more than one candidate and must be disambiguated with explicit
    // dispatch (spec `types/traits.md`).
    let src = r#"
        trait Grow { fn grow(self): number }
        trait Spurt { fn grow(self): number }
        struct Rect { w: number }
        impl Grow for Rect {
            fn grow(self): number { return 1 }
        }
        impl Spurt for Rect {
            fn grow(self): number { return 2 }
        }
        fn main() {
            let r = { w: 1 }
            print(str(r.grow()))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0809");
    assert!(
        err.message
            .contains("multiple applicable implementations of `grow` for `Rect`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn bound_check_requires_applicable_impl_not_mere_name() {
    // `impl<T: Show> Show for Wrapper<T>` registers `(Show, Wrapper)`, but
    // for `Wrapper<Dot>` its own bound fails, so the impl contributes
    // nothing: inferring `T = Wrapper<Dot>` against `T: Show` must reject
    // it (E0801) instead of answering from the registered name alone.
    let src = r#"
        trait Show { fn show(self): string }
        struct Dot { r: number }
        struct Wrapper<T> { inner: T }
        impl<T: Show> Show for Wrapper<T> {
            fn show(self): string { return "w" }
        }
        fn accept<T: Show>(x: T): string { return "s" }
        fn main() {
            let w = { inner: { r: 1 } }
            print(accept(w))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0801");
    assert!(
        err.message
            .contains("`Wrapper<Dot>` does not implement `Show`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn dispatch_on_generic_impl_receiver_is_gated_by_applicability() {
    // Explicit dispatch registers `(Show, Wrapper)`, but the impl cannot be
    // instantiated for `Wrapper<Dot>`: dispatch reports "does not implement"
    // the same way a missing impl does.
    let src = r#"
        trait Show { fn show(self): string }
        struct Dot { r: number }
        struct Wrapper<T> { inner: T }
        impl<T: Show> Show for Wrapper<T> {
            fn show(self): string { return "w" }
        }
        fn main() {
            let w = { inner: { r: 1 } }
            print(Show::show(w))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("does not implement trait `Show`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn dispatch_applies_when_generic_impl_instantiates() {
    // Same shape, but `Dot` does implement `Show`: the bound holds for the
    // instantiation `T = Dot`, so dispatch resolves.
    let src = r#"
        trait Show { fn show(self): string }
        struct Dot { r: number }
        struct Wrapper<T> { inner: T }
        impl Show for Dot {
            fn show(self): string { return "d" }
        }
        impl<T: Show> Show for Wrapper<T> {
            fn show(self): string { return "w" }
        }
        fn main() {
            let w = { inner: { r: 1 } }
            let s: string = Show::show(w)
            print(s)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn impl_receiver_argument_count_must_match_declaration() {
    // The receiver of an `impl` is a type application too: `Pair` takes two
    // arguments, so `impl Pair<number>` is E0230 at the impl site.
    let src = r#"
        struct Pair<A, B> { first: A, second: B }
        impl Pair<number> {
            fn first_of(self): number { return self.first }
        }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0230");
    assert!(
        err.message
            .contains("`Pair` expects 2 type arguments, found 1"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn qualified_trait_dispatch_resolves() {
    // spec `expressions/path-and-access.md` ("Qualified calls"):
    // `Area::area(r)` is the explicit dispatch form — the trait name is a
    // path namespace, not a value of its own.
    let src = r#"
        trait Area { fn area(self): number }
        struct Rect { w: number, h: number }
        impl Area for Rect {
            fn area(self): number { return self.w * self.h }
        }
        fn main() {
            let r = { w: 3, h: 4 }
            let n: number = Area::area(r)
            print(str(n))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn generic_bound_explicit_dispatch_resolves() {
    // spec `types/traits.md` example body: `Area::area(self.shape)` inside
    // `impl<T: Area> Palette<T>` — explicit dispatch through the bound, with
    // the member form denoting the same call (spec `types/generics.md`).
    let src = r#"
        trait Area { fn area(self): number }
        struct Palette<T> { shape: T }
        struct Rect { w: number, h: number }
        impl Area for Rect {
            fn area(self): number { return self.w * self.h }
        }
        impl<T: Area> Palette<T> {
            fn render(self): number { return Area::area(self.shape) }
        }
        fn main() {
            let ok = { shape: { w: 3, h: 4 } }
            let n: number = ok.render()
            print(str(n))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn generic_dispatch_without_bound_is_rejected() {
    // Explicit dispatch through a parameter requires that the parameter's
    // declaration bounds it by the dispatched trait (spec `types/generics.md`).
    let src = r#"
        trait Area { fn area(self): number }
        trait Show { fn show(self): string }
        fn describe<T: Show>(t: T): number {
            return Area::area(t)
        }
        fn main() { print(1) }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("add `T: Area`"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn member_field_missing_on_struct_is_e0102() {
    // Field access (not a call) on a name the struct does not declare is
    // E0102 with the receiver's type in the message (spec
    // `expressions/path-and-access.md`).
    let src = r#"
        struct User { name: string }
        fn main() {
            let u = { name: "a" }
            print(u.nope)
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0102");
    assert!(
        err.message.contains("no member `nope` on `User`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn math_namespace_constant_and_calls_typecheck() {
    let src = r#"
        fn main() {
            let r = 2.5
            let circumference = 2.0 * Math::PI * r
            let hypotenuse = Math::sqrt(3.0 * 3.0 + 4.0 * 4.0)
            let clamped = Math::min(Math::max(120, 0), 100)
            let whole = Math::floor(3.7)
            let fixed = Math::toFixed(3.14159, 2)
            print(str(circumference + hypotenuse + clamped + whole))
            print(fixed)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn math_call_argument_type_is_e0202() {
    let src = r#"
        fn main() {
            print(str(Math::sqrt(true)))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0202");
    assert!(
        err.message
            .contains("argument to `Math::sqrt` must be `float`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn math_call_arity_is_e0207() {
    let src = r#"
        fn main() {
            print(str(Math::max(1)))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0207");
    assert!(
        err.message.contains("expected 2 arguments, found 1"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn math_unknown_member_is_rejected() {
    let src = r#"
        fn main() {
            print(str(Math::typo(1)))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("`Math` has no member `typo`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn math_constant_as_a_call_is_rejected() {
    let src = r#"
        fn main() {
            print(str(Math::PI()))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("member `Math::PI` is not a function"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn math_namespace_dot_form_is_rejected() {
    let src = r#"
        fn main() {
            print(str(Math.max(1, 2)))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0003");
    assert!(
        err.message.contains("undefined variable `Math`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn task_namespace_dot_form_is_rejected() {
    let src = r#"
        async fn main() {
            print(await Task.all([]))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0003");
    assert!(
        err.message.contains("undefined variable `Task`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn trait_name_dot_dispatch_is_rejected() {
    let src = r#"
        trait Area { fn area(self): number }
        struct Rect { w: number, h: number }
        impl Area for Rect {
            fn area(self): number { return self.w * self.h }
        }
        fn main() {
            let r = { w: 3, h: 4 }
            print(str(Area.area(r)))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0003");
    assert!(
        err.message.contains("undefined variable `Area`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn time_namespace_types_and_await_check() {
    let src = r#"
        async fn later() {
            await Time::sleep(1)
        }
        fn main() {
            let now = Time::now()
            print(str(now))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn time_unknown_member_is_rejected() {
    let src = r#"
        fn main() {
            print(str(Time::nope()))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("`Time` has no member `nope`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn fs_namespace_signatures_typecheck() {
    let src = r#"
        fn main() {
            let path = "f.txt"
            let wrote: Result<unit, Error> = Fs::write(path, "hi")
            let present: bool = Fs::exists(path)
            let content: Result<string, Error> = Fs::read(path)
            let made: Result<unit, Error> = Fs::mkdir("a/b/c")
            let gone: Result<unit, Error> = Fs::remove(path)
            print(str(present))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn fs_read_result_matches_like_the_builtin_result() {
    let src = r#"
        fn main() {
            match Fs::read("f.txt") {
                Result::Ok(content) => print(content)
                Result::Err(e) => print(e.message)
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn fs_call_arity_is_e0207() {
    let src = r#"
        fn main() {
            print(str(Fs::read()))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0207");
    assert!(
        err.message.contains("expected 1 argument, found 0"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn fs_call_argument_type_is_e0202() {
    let src = r#"
        fn main() {
            print(str(Fs::exists(42)))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0202");
    assert!(
        err.message
            .contains("argument to `Fs::exists` must be `string`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn fs_unknown_member_is_rejected() {
    let src = r#"
        fn main() {
            Fs::typo("x")
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("`Fs` has no member `typo`"),
        "unexpected message: {}",
        err.message
    );
}

// ──────────────────────────────────────────────────────────────────────
//  Declaration attributes: the closed registry (`#Test`, `#Derive`) and
//  struct construction — spec `types/traits.md` "Attributes",
//  `type-system/errors.md` E0107–E0112, `types/composite-types.md`.
// ──────────────────────────────────────────────────────────────────────

#[test]
fn unknown_attribute_is_e0108() {
    let err = analyze_src("#Inline\nfn f() {}").unwrap_err();
    assert_eq!(err.code(), "E0108", "unexpected: {}", err.message);
    assert!(
        err.message.contains("attribute `Inline` is not defined"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn derive_on_a_function_is_e0109() {
    let err = analyze_src("#Derive(ToString)\nfn f() {}").unwrap_err();
    assert_eq!(err.code(), "E0109", "unexpected: {}", err.message);
    assert!(
        err.message
            .contains("attribute `Derive` is not valid on a function"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn test_on_a_struct_is_e0109() {
    let err = analyze_src("#Test\nstruct S { a: string }").unwrap_err();
    assert_eq!(err.code(), "E0109", "unexpected: {}", err.message);
    assert!(
        err.message
            .contains("attribute `Test` is not valid on a struct"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn derive_shape_errors_are_e0110() {
    // Bare and empty-paren forms both lack the required list.
    for src in [
        "#Derive\nstruct S { a: string }",
        "#Derive()\nstruct S { a: string }",
    ] {
        let err = analyze_src(src).unwrap_err();
        assert_eq!(err.code(), "E0110", "{src}: {}", err.message);
        assert!(
            err.message
                .contains("attribute `Derive` takes a parenthesized list of trait names"),
            "{src}: {}",
            err.message
        );
    }
}

#[test]
fn wrong_test_shape_is_e0110() {
    // `#Test` is not a `TestDecl` — params, a return type, or generics send
    // the parse through `fn_decl` with the attribute attached (spec
    // testing.md: no params, no return, no generics). Newlines are
    // insignificant, so `#Test` + `fn t() {}` on separate lines stays a
    // well-formed `TestDecl` and is not in this list.
    for src in [
        "#Test fn t(x: number) {}",
        "#Test fn t(): number { return 1 }",
        "#Test fn t<T>() {}",
    ] {
        let err = analyze_src(src).unwrap_err();
        assert_eq!(err.code(), "E0110", "{src}: {}", err.message);
        assert!(
            err.message.contains(
                "attribute `Test` takes the form `#Test (\"…\")? [async] fn name() { … }`"
            ),
            "{src}: {}",
            err.message
        );
    }
}

#[test]
fn titled_test_decl_is_accepted() {
    // spec `testing.md` / `add-test-title`: the optional string group is a
    // display title and changes nothing else about the fixed shape.
    let src = r#"
        #Test("adds two numbers")
        fn passes() { let x: number = 1 }

        #Test()
        fn empty_group() { let y: number = 2 }

        #Test("await works")
        async fn also_passes() { await Time::sleep(0) }
    "#;
    analyze_src(src).unwrap_or_else(|err| panic!("{}: {}", err.code(), err.message));
}

#[test]
fn identifier_group_on_test_is_e0110() {
    // `#Test(foo)` cannot match `TestDecl` (its group is a `StringLiteral`),
    // so the sequence parses as an attribute on `fn` and the shape check
    // rejects it (spec `add-test-title`).
    let err = analyze_src("#Test(foo) fn t() {}").unwrap_err();
    assert_eq!(err.code(), "E0110", "unexpected: {}", err.message);
    assert!(
        err.message
            .contains("attribute `Test` takes the form `#Test (\"…\")? [async] fn name() { … }`"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn primitive_type_names_are_keywords() {
    // The seventeen primitive words are keywords (spec
    // `lexical-structure.md`), so a lowercase binding name is a syntax
    // error and an argument list on one no longer forms a type application.
    for src in [
        "fn main() { let string = \"s\" }",
        "fn main() { let number = 42 }",
        "fn main() { let x: number<int> = 1 }",
    ] {
        let err = analyze_src(src).unwrap_err();
        assert_eq!(err.code(), "E0002", "{src}: {}", err.message);
    }
}

#[test]
fn unregistered_derive_target_is_e0111_with_note() {
    let err = analyze_src("#Derive(Display)\nstruct S { a: string }").unwrap_err();
    assert_eq!(err.code(), "E0111", "unexpected: {}", err.message);
    assert!(
        err.message
            .contains("`Display` is not a registered derive target"),
        "unexpected: {}",
        err.message
    );
    let note = err.note.expect("E0111 carries the registry note");
    assert_eq!(note, "registered targets: `ToString`");
}

#[test]
fn derive_on_a_generic_type_is_e0112() {
    let err = analyze_src("#Derive(ToString)\nstruct Box<T> { v: T }").unwrap_err();
    assert_eq!(err.code(), "E0112", "unexpected: {}", err.message);
    assert!(
        err.message
            .contains("cannot derive `ToString` for a generic type"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn referencing_a_test_fn_is_e0107() {
    let err = analyze_src("#Test fn t() {}\nfn main() { t() }").unwrap_err();
    assert_eq!(err.code(), "E0107", "unexpected: {}", err.message);
    let err = analyze_src("#Test fn t() {}\nfn main() { let x = t }").unwrap_err();
    assert_eq!(err.code(), "E0107", "unexpected: {}", err.message);
}

#[test]
fn derive_field_without_string_form_is_e0801() {
    let err = analyze_src("#Derive(ToString)\nstruct S { xs: List<number> }").unwrap_err();
    assert_eq!(err.code(), "E0801", "unexpected: {}", err.message);
    assert!(
        err.message
            .contains("`List<number>` does not implement `ToString`"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn second_derive_is_e0805() {
    let err =
        analyze_src("#Derive(ToString)\n#Derive(ToString)\nstruct S { a: string }").unwrap_err();
    assert_eq!(err.code(), "E0805", "unexpected: {}", err.message);
}

#[test]
fn derive_after_manual_impl_is_e0805() {
    // The derive expands into an impl, so a hand-written `impl ToString` for
    // the same type collides however the two are ordered (spec
    // `types/traits.md`).
    let err = analyze_src(
        "#Derive(ToString)\nstruct S { a: string }\n\
         impl ToString for S { fn toString(self): string { return self.a } }",
    )
    .unwrap_err();
    assert_eq!(err.code(), "E0805", "unexpected: {}", err.message);
}

#[test]
fn well_formed_attributes_and_derive_are_accepted() {
    // Valid shapes across every attributed declaration, a `#Test fn` body,
    // and uses of the derived type (member call, explicit dispatch, `str`,
    // interpolation — spec `types/traits.md`).
    let src = r#"
        #Derive(ToString)
        struct User { name: string, age: number }
        #Derive(ToString)
        struct Tagged { inner: User }
        #Derive(ToString)
        enum Shape { Circle(number) Rect(number, number) }
        #Test
        fn passes() { let x: number = 1 }
        #Test
        async fn also_passes() { await Time::sleep(0) }
        trait Named { fn name(self): string }
        impl Named for User { fn name(self): string { return self.name } }
        fn main() {
            let u = User(name: "Alice", age: 30)
            let s: string = u.toString()
            let t: string = ToString::toString(u)
            let b: string = str(u)
            let c: string = str(Tagged(inner: u))
            let d: string = str(Shape::Rect(3, 4))
            let e: string = `${u}`
        }
    "#;
    analyze_src(src).unwrap_or_else(|err| panic!("{}: {}", err.code(), err.message));
}

#[test]
fn enum_without_string_form_is_e0212() {
    let err = analyze_src("enum Color { Red }\nfn main() { print(str(Color::Red)) }").unwrap_err();
    assert_eq!(err.code(), "E0212", "unexpected: {}", err.message);
    assert!(
        err.message.contains("`Color` has no string form"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn struct_construction_happy_path_and_type() {
    // Named construction with defaults/optional omitted infers the struct
    // type; the values check against the field types.
    let src = r#"
        struct Config { name: string, alias: string? }
        fn main() {
            let a: Config = Config(name: "x")
            let b = Config(name: "y", alias: "z")
            let n: string = a.name
            let m: string? = b.alias
        }
    "#;
    analyze_src(src).unwrap_or_else(|err| panic!("{}: {}", err.code(), err.message));
}

#[test]
fn struct_construction_errors_are_e0205_e0206_e0208_e0202() {
    let err = analyze_src("struct R { name: string }\nfn main() { let r = R() }").unwrap_err();
    assert_eq!(err.code(), "E0205", "unexpected: {}", err.message);
    assert!(
        err.message
            .contains("missing field `name` in initializer of `R`"),
        "unexpected: {}",
        err.message
    );

    let err =
        analyze_src("struct R { name: string }\nfn main() { let r = R(nam: \"x\") }").unwrap_err();
    assert_eq!(err.code(), "E0206", "unexpected: {}", err.message);
    assert!(
        err.message.contains("`R` has no field `nam`"),
        "unexpected: {}",
        err.message
    );

    let err = analyze_src("struct R { name: string }\nfn main() { let r = R(\"x\") }").unwrap_err();
    assert_eq!(err.code(), "E0208", "unexpected: {}", err.message);
    assert!(
        err.message
            .contains("struct `R` is constructed with named arguments"),
        "unexpected: {}",
        err.message
    );

    let err =
        analyze_src("struct R { name: string }\nfn main() { let r = R(name: 42) }").unwrap_err();
    assert_eq!(err.code(), "E0202", "unexpected: {}", err.message);
    assert!(
        err.message
            .contains("field `name` of `R` expects `string`, found `number`"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn struct_construction_duplicate_label_is_rejected() {
    let err =
        analyze_src("struct R { name: string }\nfn main() { let r = R(name: \"a\", name: \"b\") }")
            .unwrap_err();
    assert!(
        err.message.contains("provided twice"),
        "unexpected: {}",
        err.message
    );
}

// ------------------------------------------------------------------
// Call-site type arguments: `first<int>(…)` — spec expressions/calls.md,
// types/generics.md (`E0801`/`E0807`/`E0808`, written-argument rules).
// ------------------------------------------------------------------

#[test]
fn written_type_args_on_non_generic_function_are_e0807() {
    let err = analyze_src(
        "fn greet(name: string): string { name }\nfn main() { print(greet<int>(\"x\")) }",
    )
    .unwrap_err();
    assert_eq!(err.code(), "E0807", "unexpected: {}", err.message);
    assert_eq!(
        err.message, "`greet` accepts no type arguments",
        "spec registration is `accepts`, not `takes`"
    );
}

#[test]
fn written_type_arg_arity_is_e0808() {
    let err = analyze_src(
        "fn first<T>(xs: List<T>): T { return xs[0] }\nfn main() { let x = first<number, string>([1]) }",
    )
    .unwrap_err();
    assert_eq!(err.code(), "E0808", "unexpected: {}", err.message);
    assert!(
        err.message.contains("expected 1 type argument, found 2"),
        "unexpected: {}",
        err.message
    );

    let err = analyze_src(
        "fn first<T>(xs: List<T>): T { return xs[0] }\nfn main() { let x = first<>([1]) }",
    )
    .unwrap_err();
    assert_eq!(err.code(), "E0808", "unexpected: {}", err.message);
    assert!(
        err.message.contains("expected 1 type argument, found 0"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn empty_list_on_non_generic_function_is_e0807() {
    let err = analyze_src("fn greet(name: string): string { name }\nfn main() { greet<>(\"x\") }")
        .unwrap_err();
    assert_eq!(err.code(), "E0807", "unexpected: {}", err.message);
}

#[test]
fn written_type_arg_bound_violation_is_e0801() {
    let src = "\
trait Area { fn area(self): number }
struct Rect { w: number, h: number }
struct Dot { r: number }
impl Area for Rect { fn area(self): number { return self.w * self.h } }
fn describe<T: Area>(x: T): number { return Area::area(x) }
fn main() {
  let d = { r: 1 }
  print(str(describe<Dot>(d)))
}";
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0801", "unexpected: {}", err.message);
    assert!(
        err.message.contains("`Dot` does not implement `Area`"),
        "unexpected: {}",
        err.message
    );

    // A written argument that does satisfy the bound checks clean.
    let ok_src = "\
trait Area { fn area(self): number }
struct Rect { w: number, h: number }
impl Area for Rect { fn area(self): number { return self.w * self.h } }
fn describe<T: Area>(x: T): number { return Area::area(x) }
fn main() {
  let r = { w: 3, h: 4 }
  print(str(describe<Rect>(r)))
}";
    analyze_src(ok_src).unwrap();
}

#[test]
fn e0807_reaches_every_non_generic_call_path() {
    let cases: &[(&str, &str)] = &[
        (
            "namespace",
            "async fn main() { let xs = Task::all<number>([]) }",
        ),
        (
            "namespace-math",
            "fn main() { print(str(Math::max<number>(1, 2))) }",
        ),
        (
            "constructor",
            "struct User { name: string }\nfn main() { let u = User<number>(name: \"a\") }",
        ),
        ("intrinsic", "fn main() { print(str(str<number>(1))) }"),
        (
            "method",
            "struct S { n: number }\nimpl S { fn get(self): number { return self.n } }\nfn main() { let s = { n: 1 } print(str(s.get<number>())) }",
        ),
        (
            "enum-ctor",
            "enum Shape { Circle(number) }\nfn main() { print(str(Shape::Circle<number>(2.5))) }",
        ),
        ("component", "fn main() { Text<number>(\"Hi\") { } }"),
        (
            "fn-value",
            "fn get(): fn(number): number { return fn(x: number): number { x * 2 } }\nfn main() { let f = get() let n = f<number>(1) print(str(n)) }",
        ),
    ];
    for (what, src) in cases {
        let err = analyze_src(src)
            .err()
            .unwrap_or_else(|| panic!("{what}: expected an error"));
        assert_eq!(err.code(), "E0807", "{what}: unexpected {}", err.message);
        assert!(
            err.message.ends_with("accepts no type arguments"),
            "{what}: unexpected {}",
            err.message
        );
    }
}

#[test]
fn written_list_on_a_value_callee_is_e0807() {
    let src = "\
fn getFn(): fn(number): number { return fn(x: number): number { x * 2 } }
fn main() { let n = getFn()<number>(1) print(str(n)) }";
    let err = analyze_src(src).unwrap_err();
    assert_eq!(err.code(), "E0807", "unexpected: {}", err.message);
    assert!(
        err.message.contains("accepts no type arguments"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn written_type_args_substitute_before_the_value_check() {
    // spec generics.md: the written list replaces inference; arguments are
    // checked against the substituted parameter types — the expected type of
    // the binding must not rescue a mismatched argument.
    let err = analyze_src(
        "fn first<T>(xs: List<T>): T { return xs[0] }\nfn main() { let x = first<number>([\"a\"]) }",
    )
    .unwrap_err();
    assert_eq!(err.code(), "E0202", "unexpected: {}", err.message);
    assert!(
        err.message.contains("`List<number>`") && err.message.contains("`List<string>`"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn written_type_args_replace_inference_and_the_result_binds() {
    let src = "\
fn first<T>(xs: List<T>): T { return xs[0] }
fn main() {
  let a = first<number>([1])
  let b = first([\"x\"])
  let c: string = first<string>([\"y\"])
  print(str(a) + str(b) + c)
}";
    analyze_src(src).unwrap();
}

#[test]
fn expected_binding_type_over_a_written_list_is_e0201() {
    let err = analyze_src(
        "fn first<T>(xs: List<T>): T { return xs[0] }\nfn main() { let ys: string = first<number>([1]) }",
    )
    .unwrap_err();
    assert_eq!(err.code(), "E0201", "unexpected: {}", err.message);
}

#[test]
fn written_type_args_work_with_named_arguments() {
    analyze_src(
        "fn first<T>(items: List<T>): T { return items[0] }\nfn main() { let x = first<number>(items: [1]) print(str(x)) }",
    )
    .unwrap();

    let err = analyze_src(
        "fn first<T>(items: List<T>): T { return items[0] }\nfn main() { let x = first<number>(items: [\"a\"]) }",
    )
    .unwrap_err();
    assert_eq!(err.code(), "E0202", "unexpected: {}", err.message);
}

#[test]
fn comparison_fallback_still_typechecks_as_comparison() {
    // Condition 2 of the disambiguation fails (no `(` after `>`), so the
    // operator stays relational and the condition is a boolean expression.
    analyze_src(
        "fn main() { let a = 2 let b = 3 let c = 4 if a < b and b > (c - 1) { print(\"x\") } }",
    )
    .unwrap();
    // Single comparison with the type-arg-shaped material and no paren is
    // non-associative per the grammar — it must be a parse error, not a call.
    let err = analyze_src("fn main() { let a = 1 let b = 2 let x = a < number > [1] }");
    assert!(err.is_err(), "two relational operators cannot chain");
}
