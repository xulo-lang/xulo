use xulo_lexer::tokenize;
use xulo_parser::parse_program;
use xulo_semantic::analyze;

fn analyze_src(src: &str) -> Result<(), xulo_core::error::XuloError> {
    let tokens = tokenize(src).unwrap();
    let program = parse_program(&tokens)?;
    analyze(&program)
}

#[test]
fn pub_declarations_registered_as_exports() {
    use xulo_semantic::analyze_with;
    let src = r#"
        pub fn add(a: number): number { return a }
        pub const PI = 3.14
        pub enum Status { Active Inactive }
        pub type User = { name: string }
    "#;
    let tokens = tokenize(src).unwrap();
    let program = parse_program(&tokens).unwrap();
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
    assert!(err.message.contains("boolean"));
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
    assert!(err.message.contains("must iterate over a `list`"));
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
    let err = analyze_src(r#"fn main() { let x: number = 1 x = "s" }"#).unwrap_err();
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
        type User = { name: string }
        fn main() {
            let u = { name: "a" }
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
        fn first<T>(list: list<T>): T { list[0] }
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
    assert!(analyze_src(r#"fn main() { print("n=" + str(null)) }"#).is_ok());
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
    assert!(analyze_src("fn main() { let c = 0 while c < 3 { c = c + 1 } }").is_ok());
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
        fn first<T>(xs: list<T>): T { xs[0] }
        let a: number = first([1, 2, 3])
        print(a)
    "#;
    assert!(analyze_src(src).is_ok());
    let err = analyze_src(
        "fn first<T>(xs: list<T>): T { xs[0] }\nlet s: string = first([1, 2])\nfn main() { print(s) }",
    )
    .unwrap_err();
    assert!(err.message.contains("cannot bind a value of type"));
}

#[test]
fn member_access_fields() {
    assert!(
        analyze_src("fn main() { let user: { name: string } = { name: \"a\" } print(user.name) }")
            .is_ok()
    );
    let err =
        analyze_src("fn main() { let user: { name: string } = { name: \"a\" } print(user.age) }")
            .unwrap_err();
    assert!(err.message.contains("no member `age`"));
}

#[test]
fn optional_member_needs_safe_access() {
    let src = "fn main() { let u: { name: string }? = null print(u?.name) }";
    assert!(analyze_src(src).is_ok());
    let err =
        analyze_src("fn main() { let u: { name: string }? = null print(u.name) }").unwrap_err();
    assert!(err.message.contains("without `?.`"));
}

#[test]
fn index_targets_lists() {
    assert!(analyze_src("fn main() { let xs: list<number> = [1, 2] print(xs[0]) }").is_ok());
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
        fn load(): async number { let v = 1 return v }
        fn main(): async { let v = await load() print(v) }
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
    let ok = analyze_src("fn main(): async { await work() }").is_ok();
    // `work` is unknown; importing seeds it, but a non-promise await is caught:
    let err = analyze_src("fn n(): number { return 1 }\nfn main(): async { let v = await n() }")
        .unwrap_err();
    assert!(err.message.contains("non-promise"));
    let _ = ok;
}

#[test]
fn try_catch_throws_are_checked() {
    let src = r#"
        fn main() {
            try { throw "boom" } catch (e) { print(e) }
            throw 1
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn imported_symbols_are_checked_against_exported_signatures() {
    let src = r#"
        import { add as sum } from "./math"
        fn main() { print(sum(1, 2)) }
    "#;
    // Without a module graph the import is opaque (Any), so it must still parse
    // and analyze successfully.
    assert!(analyze_src(src).is_ok());
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
        fn main(): async {
            let work = fn(): async { 42 }
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
        fn takes(xs: list<number>) { print(xs) }
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
    let err = analyze_src("fn f(): async number { \"s\" }").unwrap_err();
    assert!(err.message.contains("return type mismatch"));
}

#[test]
fn accepts_matching_implicit_return() {
    let src = r#"
        fn ok(): string { "s" }
        fn n(): number { 5 }
        fn xs(): list<number> { [1, 2, 3] }
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

    let err = analyze_src("fn helper(): async { @State let count = 0 }").unwrap_err();
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
        fn useAppStore(): object { return { user: null } }
        fn main(): View {
            @State let editing: boolean = false
            @Store const { user } = useAppStore()
            @Environment let router: object
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
    assert!(err.message.contains("must iterate over a `list`"));
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
        "type User = object\nfn main(): View { @State let u: User = { name: \"a\" } VStack { u.name } }"
    )
    .is_ok());

    // The documented custom-component pattern: a `children: list<View>`
    // parameter is forwarded as a bare element.
    let src = r#"
        fn MyCard(title: string, children: list<View>): View {
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
        "fn main(): View { @State let ok: boolean = true VStack { ok } }",
        "fn main(): View { @State let xs: list<number> = [1, 2] VStack { xs } }",
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
        "fn work(): async { 42 }\nfn main(): async { let ok = true let x = if ok { await work() } else { 0 } }",
    )
    .unwrap_err();
    assert!(err.message.contains("`if`/`match` expression"));
}

#[test]
fn await_rejected_inside_value_position_match() {
    let err = analyze_src(
        "fn work(): async { 42 }\nfn main(): async { let x = match 1 { 1 => await work() _ => 0 } }",
    )
    .unwrap_err();
    assert!(err.message.contains("`if`/`match` expression"));
}

#[test]
fn await_rejected_inside_implicit_return_if() {
    let err = analyze_src(
        "fn work(): async { 42 }\nfn main(): async { let ok = true if ok { await work() } else { 0 } }",
    )
    .unwrap_err();
    assert!(err.message.contains("`if`/`match` expression"));
}

#[test]
fn await_allowed_in_statement_position_if() {
    let src = r#"
        fn work(): async { 42 }
        fn main(): async {
            let ok = true
            if ok { let v = await work() print(v) }
            print(7)
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn await_allowed_inside_ternary() {
    let src = "fn work(): async { 42 }\nfn main(): async { let ok = true let x = ok ? await work() : 0 print(x) }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn await_rejected_in_non_async_closure_inside_async_fn() {
    let err =
        analyze_src("fn work(): async { 42 }\nfn main(): async { let g = fn() { await work() } }")
            .unwrap_err();
    assert!(err.message.contains("inside an `async` function"));
}

#[test]
fn await_allowed_in_async_closure() {
    let src = "fn main(): async { let g = fn(): async { 42 } let v = await g() print(v) }";
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
        fn useAppStore(): object { return { user: null } }
        fn main(): View {
            @State let count: number = 0
            @Store const { user } = useAppStore()
            @Environment let router: object
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
    let tokens = tokenize("fn main() { print(1); }").unwrap();
    let program = parse_program(&tokens).unwrap();
    let result = xulo_semantic::analyze_with(&program, &[], &[], &[]).unwrap();
    assert!(
        result.warnings.is_empty(),
        "unexpected warnings: {:?}",
        result.warnings
    );
}

#[test]
fn implicit_return_warning_only_with_declared_return_type() {
    let tokens = tokenize("fn f(): number { 1; }").unwrap();
    let program = parse_program(&tokens).unwrap();
    let result = xulo_semantic::analyze_with(&program, &[], &[], &[]).unwrap();
    assert_eq!(result.warnings.len(), 1, "warnings: {:?}", result.warnings);
    assert!(result.warnings[0].message.contains("ignored return value"));
}

#[test]
fn rejects_unary_not_on_non_boolean() {
    let err = analyze_src("fn main() { print(!5) }").unwrap_err();
    assert!(err.message.contains("unary `!` requires a `boolean`"));
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
    // silently producing a `list<number>` whose runtime value is corrupted.
    let err =
        analyze_src(r#"fn main() { let xs: list<number> = [1, "a"] print(xs) }"#).unwrap_err();
    assert!(
        err.message
            .contains("cannot bind a value of type `list<number | string>`"),
        "got: {}",
        err.message
    );
    // An unannotated mixed literal infers a union element type...
    assert!(
        analyze_src(r#"fn main() { let xs = [1, "a"] print(xs) }"#).is_ok(),
        "unannotated mixed list must infer `list<number | string>`"
    );
    // ...and an explicit union annotation is the legal escape hatch.
    assert!(
        analyze_src(r#"fn main() { let xs: list<number | string> = [1, "a"] print(xs) }"#).is_ok()
    );
    // Spread elements participate: `list<string>` into a numeric list yields
    // a union, which a `list<number>` binding still rejects.
    let err = analyze_src(
        r#"fn main() { let strs: list<string> = ["x"] let xs: list<number> = [1, ...strs] print(xs) }"#,
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
    // Closures get the same treatment.
    assert!(
        analyze_src(
            "fn main() { let g = fn(a: number, b: number = a): number { b } print(str(g(1))) }"
        )
        .is_ok()
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
        type Rect = object
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
        fn pause(): async { }
        fn g(): async number { await pause() 1 }
        fn main(): async {
            let x = if true {
                fn inner(): async number { await g() }
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
fn member_assignment_is_type_checked_on_typed_object() {
    assert!(
        analyze_src("fn main() { let u: { age: number } = { age: 1 } u.age = 2 print(u.age) }")
            .is_ok()
    );
    let err =
        analyze_src("fn main() { let u: { age: number } = { age: 1 } u.age = \"x\" print(u.age) }")
            .unwrap_err();
    assert!(
        err.message
            .contains("cannot assign a value of type `string` to `u.age")
    );
}

#[test]
fn unknown_member_assignment_is_rejected() {
    let err =
        analyze_src("fn main() { let u: { age: number } = { age: 1 } u.missing = 2 }").unwrap_err();
    assert!(err.message.contains("no member `missing`"));
}

#[test]
fn index_assignment_is_type_checked_on_typed_list() {
    assert!(analyze_src("fn main() { let xs: list<number> = [1] xs[0] = 5 print(xs) }").is_ok());
    let err = analyze_src("fn main() { let xs: list<number> = [1] xs[0] = \"s\" print(xs) }")
        .unwrap_err();
    assert!(
        err.message
            .contains("cannot assign a value of type `string` to `xs[...]: number`")
    );
}

#[test]
fn non_list_index_assignment_is_rejected() {
    let err = analyze_src("fn main() { print(5) let n: number = 1 n[0] = 2 }").unwrap_err();
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
        fn same(a: list<number>?, b: list<number>?): boolean { a == b }
        fn main() { print(same(null, null)) print(same([1], [1])) }
    "#;
    assert!(analyze_src(ok).is_ok());
    // An optional list cannot be indexed directly (no narrowing).
    let err = analyze_src("fn g(xs: list<number>?): number { xs[0] }").unwrap_err();
    assert!(err.message.contains("cannot index into `list<number>?`"));
}

#[test]
fn nested_generic_list_indexing_types_check() {
    let src =
        "fn main() { let m: list<list<number>> = [[1, 2], [3]] let n: number = m[0][1] print(n) }";
    assert!(analyze_src(src).is_ok());
    let err = "fn main() { let m: list<list<number>> = [[1, 2], [3]] let s: string = m[0][1] }";
    assert!(analyze_src(err).is_err());
}

#[test]
fn nested_typed_object_member_access() {
    let src = "fn main() { let u: { a: { b: string } } = { a: { b: \"hi\" } } print(u.a.b) }";
    assert!(analyze_src(src).is_ok());
    let err = "fn main() { let u: { a: { b: string } } = { a: { b: \"hi\" } } print(u.a.c) }";
    assert!(
        analyze_src(err)
            .unwrap_err()
            .message
            .contains("no member `c`")
    );
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
        fn makeStore(): object { let o = { theme: { x: 1 } } o }
        fn main(): View { @Store const { theme } = makeStore() theme = { x: 2 } VStack { Text("x") } }
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
fn catch_binding_is_scoped_to_catch_block() {
    let src = "fn main() { try { throw 1 } catch (e) { print(e) } print(e) }";
    let err = analyze_src(src).unwrap_err();
    assert!(err.message.contains("undefined variable `e`"));
    assert!(analyze_src("fn main() { try { throw 1 } catch (e) { print(e) } }").is_ok());
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
        fn first<T>(xs: list<T>): T { xs[0] }
        fn main() { let n: number = first([1, 2]) print(n) }
    "#;
    assert!(analyze_src(src).is_ok());
    let err = r#"
        fn first<T>(xs: list<T>): T { xs[0] }
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
    let err = analyze_src("return 1").unwrap_err();
    assert!(err.message.contains("top level of a function body"));
    let err = analyze_src("return").unwrap_err();
    assert!(err.message.contains("top level of a function body"));
    // Bare `return;` inside a function is still fine.
    assert!(analyze_src("fn f() { if true { return } }").is_ok());
}

#[test]
fn structural_objects_require_fields() {
    // A source object that lacks a required field is not assignable.
    let err = r#"
        fn take(p: { name: string }): string { p.name }
        fn main() { let x: { age: number } = { age: 3 } take(x) }
    "#;
    let err = analyze_src(err).unwrap_err();
    assert!(
        err.message.contains("argument to `take`"),
        "unexpected {}",
        err.message
    );

    // Extra source fields are allowed (structural widening).
    let ok = r#"
        fn take(p: { name: string }): string { p.name }
        fn main() { let x: { name: string, age: number } = { name: "a", age: 3 } take(x) }
    "#;
    assert!(analyze_src(ok).is_ok());
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
        fn work(): async { 42 }
        fn main(): async {
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
        type Circle = object
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
        type Circle = object
        type Square = object
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
        type Circle = object
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
        type Circle = object
        impl Shape for Circle {
            fn area(self): number { return 100 }
        }
        fn apply<T: Shape>(t: T): number {
            // `Shape::area(t)` cannot dispatch: `T` is a generic parameter and
            // resolves only at run time; the receiver must be a concrete type.
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
        type Circle = object
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
        type c = object
        type b_c = object
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
        type Circle = object
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
        type Circle = object
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
fn store_binding_keeps_inferred_type() {
    // `@Store const n = 5` binds `n` as `number` (not `Any`), so type errors
    // on it are caught like they are for `@State`.
    assert!(
        analyze_src("fn main(): View { @Store const n = 5 VStack { Text(str(n + 1)) } }").is_ok()
    );
    let err = analyze_src("fn main(): View { @Store const n = \"x\" VStack { Text(str(n + 1)) } }")
        .unwrap_err();
    assert!(
        err.message.contains("cannot apply `+`"),
        "got: {}",
        err.message
    );
}

#[test]
fn enum_name_can_be_reexported() {
    // `pub use { Color }` re-exports an enum (which has runtime value); a type
    // alias cannot be re-exported by bare name and gets an honest message.
    assert!(
        analyze_src("enum Color { Red Blue }\npub use { Color }\nfn main() { print(1) }").is_ok()
    );
    let err =
        analyze_src("type Rect = object\npub use { Rect }\nfn main() { print(1) }").unwrap_err();
    assert!(
        err.message.contains("no runtime value"),
        "got: {}",
        err.message
    );
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
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        fn apply<T: Shape>(t: T): number {
            let area = t.area
            return area()
        }
        fn main() {
            let c: object = { area: fn(): number { return 5 } }
            print(apply(c))
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn bounded_generic_missing_bound_member_is_rejected() {
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
    assert!(
        err.message.contains("has no member `volume`"),
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
        type Circle = object
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
        type Circle = object
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
        type Circle = object
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
        type Circle = object
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
        type Circle = object
        impl Shape for Circle {
            fn area(self): async number { return 1 }
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
            fn area(self): async number
        }
        type Circle = object
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
        type C = object
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
        type Circle = object
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
            fn area(self): number
            fn perimeter(self): number
        }
        type Rect = object
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
fn generic_trait_bound_method_call_is_rejected() {
    // `s.area()` inside `T: Shape` would be a trait dispatch, but impl
    // selection needs a concrete receiver type, so it is rejected at compile
    // time instead of crashing at run time.
    let src = r#"
        trait Shape {
            fn area(self): number
        }
        fn area_of<T: Shape>(s: T): number {
            return s.area()
        }
        type Circle = object
        impl Shape for Circle {
            fn area(self): number { return 5 }
        }
        fn main() {
            let c = {}
            print(area_of(c))
        }
    "#;
    let err = analyze_src(src).unwrap_err();
    assert!(
        err.message.contains("cannot call `T`'s method `area`"),
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
    let tokens = tokenize(src).unwrap();
    let program = parse_program(&tokens).unwrap();
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
        type Shape = object
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
        type Circle = object
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
        type Circle = object
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
        type Circle = object
        impl Shape for Circle {
            fn area(self): number { return self.radius }
        }
        fn main() {
            let c: Circle = { radius: 5 }
            Shape::area(c)
        }
    "#;
    let tokens = tokenize(src).unwrap();
    let mut program = parse_program(&tokens).unwrap();
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
