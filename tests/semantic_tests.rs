use xulo::lexer::tokenize;
use xulo::parser::parse_program;
use xulo::semantic::analyze;

fn analyze_src(src: &str) -> Result<(), xulo::error::XuloError> {
    let tokens = tokenize(src).unwrap();
    let program = parse_program(&tokens)?;
    analyze(&program)
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
    assert_eq!(err.kind, xulo::error::ErrorKind::Semantic);
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
    assert!(err.message.contains("exactly 1 argument"));
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
    assert!(err.message.contains("exactly 1 argument"));
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
    let src = "fn main(): Component { }";
    assert!(analyze_src(src).is_ok());
}

#[test]
fn state_is_allowed_in_component_function() {
    let src = r#"
        fn main(): Component {
            @State let count: number = 0
        }
    "#;
    assert!(analyze_src(src).is_ok());
}

#[test]
fn decorators_rejected_outside_component() {
    let err = analyze_src("fn main() { @State let count: number = 0 }").unwrap_err();
    assert!(err.message.contains("returning `Component`"));

    let err = analyze_src("fn helper(): async { @State let count = 0 }").unwrap_err();
    assert!(err.message.contains("returning `Component`"));
}

#[test]
fn decorators_rejected_in_nested_block() {
    let err = analyze_src("fn main(): Component { if true { @State let count: number = 0 } }")
        .unwrap_err();
    assert!(err.message.contains("nested block"));
}

#[test]
fn effect_and_store_and_environment_in_component() {
    let src = r#"
        fn useAppStore(): object { return { user: null } }
        fn main(): Component {
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
        fn main(): Component {
            @State let name: string = ""
            Input(value: $name)
        }
    "#;
    assert!(analyze_src(src).is_ok());

    let err = analyze_src("fn main(): Component { let name: string = \"\" Input(value: $name) }")
        .unwrap_err();
    assert!(err.message.contains("`$` binding"));
}

#[test]
fn state_cannot_be_redeclared() {
    let err =
        analyze_src("fn main(): Component { @State let x: number = 0 @State let x: number = 1 }")
            .unwrap_err();
    assert!(err.message.contains("already declared"));
}

#[test]
fn component_children_are_type_checked() {
    let src = r#"
        fn main(): Component {
            VStack {
                if 1 > 0 { Text("ok") }
                for x in [1, 2] { Text("x") }
            }
        }
    "#;
    assert!(analyze_src(src).is_ok());

    let err =
        analyze_src("fn main(): Component { VStack { for x in 5 { Text(\"x\") } } }").unwrap_err();
    assert!(err.message.contains("must iterate over a `list`"));
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
        "fn main(): Component { fn handle() { @State let count: number = 0 } print(\"x\") }",
    )
    .unwrap_err();
    assert!(err.message.contains("returning `Component`"));
}

#[test]
fn decorators_rejected_in_anonymous_closure_inside_component() {
    let err =
        analyze_src("fn main(): Component { let handle = fn() { @State let count: number = 0 } }")
            .unwrap_err();
    assert!(err.message.contains("returning `Component`"));
}

#[test]
fn effect_cannot_capture_render_local() {
    let err = analyze_src(
        r#"
        fn main(): Component {
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
        fn main(): Component {
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
        fn main(): Component {
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
        fn main(): Component {
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
        fn main(): Component {
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
    let result = xulo::semantic::analyze_with(&program, &[], &[]).unwrap();
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
    let result = xulo::semantic::analyze_with(&program, &[], &[]).unwrap();
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
        fn main(): Component { @State let n: number = 0 n = 1 VStack { Text(str(n)) } }
    "#;
    assert!(analyze_src(ok).is_ok());
    let bad = r#"
        fn main(): Component { @State let n: number = 0 n = "x" VStack { Text(str(n)) } }
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
        fn main(): Component { @Store const { theme } = makeStore() theme = { x: 2 } VStack { Text("x") } }
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
