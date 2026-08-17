use xulo_codegen::generate;
use xulo_lexer::tokenize;
use xulo_parser::parse_program;

/// Full pipeline through codegen, applying the semantic phase's out-of-band
/// annotations (trait dispatch, list concat) so generated JS matches what the
/// real compiler emits.
fn generate_js(src: &str) -> String {
    let tokens = tokenize(src).unwrap();
    let mut program = parse_program(&tokens).unwrap();
    let result = xulo_semantic::analyze_with(&program, &[], &[], &[]).unwrap();
    xulo_semantic::apply_trait_dispatch(&mut program, &result.trait_dispatch);
    xulo_semantic::apply_list_concat(&mut program, &result.list_concat);
    generate(&program).unwrap()
}

#[test]
fn function_and_call() {
    let js = generate_js(
        r#"fn add(a: number, b: number): number { return a + b } fn main() { print(add(1, 2)) }"#,
    );
    assert!(js.contains("function add(a, b) {"));
    assert!(js.contains("return (a + b);"));
    assert!(js.contains("console.log(add(1, 2));"));
}

#[test]
fn trait_dispatch_emits_mangled_impl_call() {
    let js = generate_js(
        r#"
        trait Area { fn area(self): number }
        type Rectangle = object
        impl Area for Rectangle {
            fn area(self): number { return self.w * self.h }
        }
        fn rect(): Rectangle { let r = { w: 3, h: 4 } r }
        fn main() { print(Area::area(rect())) }
        "#,
    );
    assert!(js.contains("__impls[\"impl_Area_Rectangle_area\"] = function (self) {"));
    assert!(js.contains("__impls[\"impl_Area_Rectangle_area\"](rect())"));
    assert!(js.contains("const __impls = {};"));
}

#[test]
fn list_plus_list_emits_concat() {
    let js = generate_js(r#"fn main() { print([1, 2] + [3, 4]) }"#);
    assert!(js.contains("([1, 2]).concat([3, 4])"), "js:\n{js}");
    // Numeric addition keeps the plain `+`.
    let js = generate_js(r#"fn main() { print(1 + 2) }"#);
    assert!(js.contains("(1 + 2)"), "js:\n{js}");
}

#[test]
fn equality_emits_structural_eq_helper() {
    let js = generate_js(
        r#"
        enum Result { Ok(number) Err(string) }
        fn main() {
            let a = Result::Ok(1)
            let b = Result::Ok(1)
            print(str(a == b))
            print(str(a != b))
        }
        "#,
    );
    // Enum values compile to `{ tag, value }` objects; comparison must go
    // through the structural `__eq` helper (a bare `==` would be reference
    // equality and always false for separately-constructed values).
    assert!(js.contains("__eq(a, b)"), "js:\n{js}");
    assert!(js.contains("(!__eq(a, b))"), "js:\n{js}");
    assert!(js.contains("function __eq"), "js:\n{js}");
    // Plain numeric comparisons stay native.
    let js = generate_js(r#"fn main() { print(1 == 2) }"#);
    assert!(js.contains("__eq(1, 2)"), "js:\n{js}");
}

#[test]
fn print_becomes_console_log() {
    let js = generate_js(r#"fn main() { print("hi") }"#);
    assert!(js.contains("console.log(\"hi\");"));
}

#[test]
fn main_is_invoked() {
    let js = generate_js("fn main() { }");
    assert!(js.contains("main();"));
}

#[test]
fn for_loop_uses_for_of() {
    let js = generate_js(r#"fn main() { for item in [1, 2] { print(item) } }"#);
    // The loop variable is mutable (reassignable, like `let`), so codegen
    // must not emit `const` (which would throw on `item = ...` in the body).
    assert!(js.contains("for (let item of [1, 2]) {"));
}

#[test]
fn if_else_emits_statement() {
    let js = generate_js("fn main() { if true { print(1) } else { print(2) } }");
    assert!(js.contains("if (true) {"));
    assert!(js.contains("} else {"));
}

#[test]
fn numbers_are_compact() {
    let js = generate_js("fn main() { print(3.0) }");
    assert!(js.contains("console.log(3);"));
}

#[test]
fn strings_are_escaped() {
    let js = generate_js("fn main() { print(\"a\\nb\\\"c\") }");
    assert!(js.contains("\\\""));
}

#[test]
fn else_if_chain() {
    let js = generate_js("fn main() { if true { 1 } else if false { 2 } else { 3 } }");
    assert!(js.contains("} else if (false) {"));
}

#[test]
fn const_emits_const() {
    let js = generate_js("const X = 5\nfn main() { print(X) }");
    assert!(js.contains("const X = 5;"));
}

#[test]
fn assignment_emits_assignment() {
    let js = generate_js("fn main() { let x = 0 x = x + 1 print(x) }");
    assert!(js.contains("x = (x + 1);"));
}

#[test]
fn null_literal_emits_null() {
    let js = generate_js("fn main() { print(null) }");
    assert!(js.contains("console.log(null);"));
}

#[test]
fn type_aliases_are_erased() {
    let js = generate_js("type User = { name: string }\nfn main() { print(\"hi\") }");
    assert!(!js.contains("User ="));
    assert!(js.contains("console.log(\"hi\");"));
}

#[test]
fn enum_def_and_reference() {
    let js = generate_js("enum Theme { Light Dark }\nfn main() { print(Theme::Dark) }");
    assert!(js.contains("const Theme = Object.freeze("));
    assert!(js.contains("console.log(Theme.Dark);"));
}

#[test]
fn payload_enum_constructor() {
    let js = generate_js(
        "enum Result<T> { Success(T) Error(string) }\nfn main() { print(Result::Success(1)) }",
    );
    assert!(js.contains("const Result = { Success: (value) =>"));
    assert!(js.contains("console.log(Result.Success(1));"));
}

#[test]
fn let_without_value() {
    let js = generate_js("fn main() { let x: number print(x) }");
    assert!(js.contains("let x;"));
}

#[test]
fn default_params_emit_defaults() {
    let js = generate_js("fn greet(name: string = \"x\") { }\nfn main() { greet() }");
    assert!(js.contains("function greet(name = \"x\") {"));
}

#[test]
fn while_loop_emits_while() {
    let js = generate_js("fn main() { let c = 0 while c < 3 { c = c + 1 } }");
    assert!(js.contains("while ((c < 3)) {"));
}

#[test]
fn range_for_emits_c_style_loop() {
    let js = generate_js("fn main() { for i in 0..<5 { print(i) } }");
    assert!(js.contains("for (let i = 0; i < 5; i++) {"));
}

#[test]
fn match_emits_iffy() {
    let js = generate_js(r#"fn main() { let s = match 2 { 0 => "zero" _ => "other" } }"#);
    assert!(js.contains("const __m = 2;"));
    assert!(js.contains("if (__m === 0) {"));
    assert!(js.contains("return \"zero\";"));
}

#[test]
fn match_enum_payload_emits_tag_check() {
    let js = generate_js(
        "enum Result<T> { Success(T) Error(string) }\nfn main() { let r = Result::Success(1) let v = match r { Result::Success(n) => n _ => 0 } }",
    );
    assert!(js.contains("if (__m && __m.tag === \"Success\") {"));
    assert!(js.contains("const n = __m.value;"));
}

#[test]
fn ternary_emits_ternary() {
    let js = generate_js(r#"fn main() { print(1 > 2 ? "a" : "b") }"#);
    assert!(js.contains("((1 > 2) ? \"a\" : \"b\")"));
}

#[test]
fn logical_and_or_emits_js_operators() {
    let js = generate_js("fn main() { print(true and false or true) }");
    assert!(js.contains("&&"));
    assert!(js.contains("||"));
}

#[test]
fn member_and_index_emit_dot_and_brackets() {
    let js = generate_js("fn main() { let xs = [10, 20] print(xs[0]) }");
    assert!(js.contains("console.log(xs[0]);"));
}

#[test]
fn method_call_emits_receiver() {
    let js = generate_js("fn go(store: object) { store.actions.setLoading(true) }\nfn main() { }");
    assert!(js.contains("store.actions.setLoading(true);"));
}

#[test]
fn nullish_and_optional_chain_emit_js() {
    let js =
        generate_js("fn main() { let u: { name: string }? = null print(u?.name ?? \"anon\") }");
    assert!(js.contains("u?.name"));
    assert!(js.contains("?? \"anon\""));
}

#[test]
fn object_spread_emits_ellipsis() {
    let js = generate_js("fn main() { let base = { x: 1 }; let o = { ...base, y: 2 } }");
    assert!(js.contains("{...base, \"y\": 2}"));
}

#[test]
fn named_args_reordered_by_params() {
    let js = generate_js(
        "fn greet(name: string = \"x\", count: number): string { name }\nfn main() { print(greet(count: 2, name: \"hi\")) }",
    );
    assert!(js.contains("console.log(greet(\"hi\", 2));"));
}

#[test]
fn unary_not_emits_bang() {
    let js = generate_js("fn main() { print(!false) }");
    assert!(js.contains("console.log((!false));"));
}

#[test]
fn string_concat_emits_plus() {
    let js = generate_js(r#"fn main() { print("a" + "b") }"#);
    assert!(js.contains("console.log((\"a\" + \"b\"));"));
}

#[test]
fn async_fn_emits_async_function_and_await() {
    let js = generate_js(
        "fn load(): async number { let v = 1 return v }\n\
         fn main(): async { let v = await load() print(v) }",
    );
    assert!(js.contains("async function load()"));
    assert!(js.contains("async function main()"));
    assert!(js.contains("(await load())"));
}

#[test]
fn try_catch_throw_emit_native_js() {
    let js = generate_js(r#"fn main() { try { throw "boom" } catch (e) { print("caught") } }"#);
    assert!(js.contains("try {"));
    assert!(js.contains("throw \"boom\";"));
    assert!(js.contains("} catch (e) {"));
}

#[test]
fn export_decl_emits_underlying_decl() {
    let js = generate_js("export fn add(a: number): number { return a }\nfn main() {}");
    assert!(js.contains("function add(a)"));
}

#[test]
fn closure_emits_function_expression() {
    let js =
        generate_js("fn main() { let double = fn(x: number): number { x * 2 } print(double(3)) }");
    assert!(js.contains("(function (x) {"));
    assert!(js.contains("return (x * 2);"));
}

#[test]
fn async_closure_emits_async_function() {
    let js = generate_js("fn main(): async { let f = fn(): async { 42 } let v = await f() }");
    assert!(js.contains("(async function () {"));
}

#[test]
fn list_spread_emits_ellipsis() {
    let js = generate_js("fn main() { let base = [1, 2] let xs = [...base, 3] }");
    assert!(js.contains("[...base, 3]"));
}

#[test]
fn call_value_emits_parenthesized_callee() {
    let js = generate_js("fn main() { let xs = [fn(x: number): number { x }] print(xs[0](1)) }");
    assert!(js.contains("(xs[0])(1)"));
}

#[test]
fn state_emits_signal_and_get_set() {
    let js = generate_js(
        "fn main(): Component { @State let count: number = 0 count = count + 1 print(count) }",
    );
    assert!(js.contains("__signal"));
    assert!(js.contains("const count = __signal(0);"));
    assert!(js.contains("count.set((count.get() + 1));"));
    assert!(js.contains("console.log(count.get());"));
}

#[test]
fn component_emits_props_and_children() {
    let js = generate_js("fn main(): Component { VStack(spacing: 16) { Text(\"Hi\") } }");
    assert!(js.contains("__component"));
    assert!(js.contains("VStack({"));
    assert!(js.contains("children: [Text({"));
}

#[test]
fn component_emits_runtime_preamble() {
    let js = generate_js("fn main(): Component { }");
    assert!(js.contains("__signal = __runtime.signal"));
    assert!(js.contains("__component = __runtime.component"));
}

#[test]
fn expr_child_emits_direct_value() {
    let js =
        generate_js("fn main(): Component { @State let name: string = \"Xulo\" VStack { name } }");
    assert!(js.contains("children: [name.get()]"), "js: {js}");
}

#[test]
fn forwarded_children_emit_nested_array() {
    let js = generate_js("fn MyCard(children: list<Component>): Component { VStack { children } }");
    assert!(js.contains("children: [children]"), "js: {js}");
}

#[test]
fn local_component_is_called_positionally() {
    // A locally-defined component function is called with reordered positional
    // arguments and the `children` array routed to its `children` parameter
    // (external `@xulo/ui` components keep the props-object convention).
    let js = generate_js(
        r#"
        fn MyCard(title: string, children: list<Component>): Component {
            VStack { Text(title) }
        }
        fn main(): Component { MyCard(title: "Hello") { Text("Hi") } }
        "#,
    );
    assert!(js.contains("return MyCard(\"Hello\", [Text({"), "js: {js}");
}

#[test]
fn no_runtime_without_reactive_features() {
    let js = generate_js("fn main() { print(1) }");
    assert!(!js.contains("__runtime"));
}

#[test]
fn store_emits_destructure() {
    let js = generate_js(
        "fn useAppStore(): object { return { user: null } }\n\
         fn main(): Component { @Store const { user, theme } = useAppStore() }",
    );
    assert!(js.contains("const { user, theme } = useAppStore();"));
}

#[test]
fn effect_emits_runtime_call() {
    let js = generate_js(
        "fn main(): Component { @State let id: number = 0 @Effect fn() { print(id) }, [id] }",
    );
    assert!(js.contains("__effect("));
    assert!(js.contains("() => [id.get()]"));
    assert!(js.contains("function sameDeps"));
}

#[test]
fn effect_without_deps_emits_undefined_thunk() {
    let js = generate_js(
        "fn main(): Component { @State let id: number = 0 @Effect fn() { print(id) } }",
    );
    assert!(js.contains("__effect((function"));
    assert!(js.contains(", undefined);"));
}

#[test]
fn environment_emits_env_lookup() {
    let js = generate_js(
        "type Router = object\nfn main(): Component { @Environment let router: Router }",
    );
    assert!(js.contains("const router = __env(\"Router\");"));
}

#[test]
fn dollar_binding_emits_value_onchange() {
    let js =
        generate_js("fn main(): Component { @State let name: string = \"\" Input(value: $name) }");
    assert!(js.contains("{ value: name.get(), onChange: (__v) => name.set(__v) }"));
}

#[test]
fn ui_if_and_for_emit_spread() {
    let js = generate_js(
        "fn main(): Component { let ok = true let xs = [1] VStack { if ok { Text(\"a\") } for x in xs { Text(x) } } }",
    );
    assert!(js.contains("...(() => { if (ok) {"));
    assert!(js.contains(").map((x) =>"));
}

#[test]
fn component_main_emits_mount_hook() {
    let js = generate_js("fn main(): Component { }");
    assert!(js.contains("const __xulo_main = main();"));
    assert!(js.contains("__xulo_mount"));
}

#[test]
fn script_main_does_not_emit_mount_hook() {
    let js = generate_js("fn main() { print(1) }");
    assert!(js.contains("main();"));
    assert!(!js.contains("__xulo_mount"));
}

#[test]
fn closure_implicit_return_keeps_body_statements() {
    let js = generate_js(
        r#"
        fn apply(f: fn(x: number): number, v: number): number { return f(v) }
        fn main() { let g = fn(x: number): number { let n = 2 x + n } print(apply(g, 1)) }
    "#,
    );
    assert!(js.contains("let n = 2;"));
    assert!(js.contains("return (x + n);"));
}

#[test]
fn closure_implicit_return_with_range_helper() {
    let js = generate_js(
        r#"
        fn main() { let g = fn(): number { let n = 0 let r = 5..<10 r[0] } print(g()) }
    "#,
    );
    assert!(js.contains("function range(a, b) {"));
    assert!(js.contains("return r[0];"));
}

#[test]
fn optional_param_without_default_emits_null() {
    let js = generate_js(
        "fn greet(name: string?): string { return name ?? \"world\" } fn main() { print(greet()) }",
    );
    assert!(js.contains("function greet(name = null) {"));
}

#[test]
fn optional_param_with_default_keeps_default() {
    let js = generate_js(
        "fn greet(name: string? = \"xulo\"): string { return name } fn main() { print(greet()) }",
    );
    assert!(js.contains("function greet(name = \"xulo\") {"));
}

#[test]
fn closure_optional_param_emits_null() {
    let js =
        generate_js("fn main() { let g = fn(x: number?): number { return x ?? 0 } print(g(5)) }");
    assert!(js.contains("(x = null)"));
}

#[test]
fn for_var_shadows_signal() {
    let js = generate_js(
        "fn main(): Component { @State let x: number = 0 let ys = [1, 2, 3] for x in ys { print(x) } }",
    );
    assert!(js.contains("const x = __signal(0);"));
    assert!(js.contains("for (let x of ys) {"));
    assert!(js.contains("console.log(x);"));
    assert!(
        !js.contains("x.get()"),
        "loop variable must shadow the signal:\n{js}"
    );
}

#[test]
fn block_local_shadows_signal() {
    let js = generate_js(
        "fn main(): Component { @State let x: number = 0 { let x = 42 print(x) } print(x) }",
    );
    assert!(
        js.contains("console.log(x);"),
        "inner block read is a plain local:\n{js}"
    );
    assert!(
        js.contains("console.log(x.get());"),
        "outer read is still the signal:\n{js}"
    );
}

#[test]
fn ui_for_var_shadows_signal() {
    let js = generate_js(
        "fn main(): Component { @State let item: object = { name: \"x\" } let items = [{ name: \"a\" }] VStack { for item in items { Card(title: item.name) } } }",
    );
    assert!(js.contains(".map((item) =>"));
    assert!(
        js.contains("\"title\": item.name"),
        "UI for variable must shadow the signal:\n{js}"
    );
    assert!(!js.contains("item.get().name"));
}

#[test]
fn signal_used_inside_plain_fn_body() {
    let js = generate_js(
        "fn main(): Component { @State let n: number = 0 let read = fn(): number { n } VStack { } }",
    );
    assert!(
        js.contains("return n.get();"),
        "closure still sees the signal through outer scope:\n{js}"
    );
}

#[test]
fn export_main_is_invoked() {
    let js = generate_js("export fn main() { print(1) }");
    assert!(js.contains("function main()"));
    assert!(js.contains("main();"));
}

#[test]
fn export_default_main_is_invoked() {
    let js = generate_js("export default fn main() { print(1) }");
    assert!(js.contains("function main()"));
    assert!(js.contains("main();"));
}

#[test]
fn export_default_component_main_emits_mount_hook() {
    let js = generate_js("export default fn main(): Component { }");
    assert!(js.contains("const __xulo_main = main();"));
    assert!(js.contains("__xulo_mount"));
}

#[test]
fn unary_minus_emits_negated_literal() {
    let js = generate_js("fn main() { let a = -5 print(a) }");
    assert!(js.contains("let a = (-5);"));
}

#[test]
fn double_negation_emits_nested_bang() {
    let js = generate_js("fn main() { let flag = true let b = !!flag print(b) }");
    assert!(js.contains("let b = (!(!flag));"));
}

#[test]
fn index_assignment_emits_bracket_assignment() {
    let js = generate_js("fn main() { let xs: list<number> = [1] xs[0] = 10 print(xs[0]) }");
    assert!(js.contains("xs[0] = 10;"));
    assert!(js.contains("console.log(xs[0]);"));
}

#[test]
fn member_assignment_emits_dot_assignment() {
    let js = generate_js(
        "fn main() { let user: { age: number } = { age: 20 } user.age = 30 print(user.age) }",
    );
    assert!(js.contains("user.age = 30;"));
    assert!(js.contains("console.log(user.age);"));
}

#[test]
fn nested_object_literal_emits_js_object() {
    let js = generate_js(r#"fn main() { let o = { a: { b: [1, 2] }, c: "x" } print(o) }"#);
    assert!(js.contains(r#"let o = {"a": {"b": [1, 2]}, "c": "x"};"#));
}

#[test]
fn nested_ternary_emits_nested_ternary() {
    let js = generate_js("fn main() { let t = true ? false ? 3 : 4 : 5 print(t) }");
    assert!(js.contains("(true ? (false ? 3 : 4) : 5)"));
}

#[test]
fn string_relational_emits_js_comparison() {
    let js = generate_js(r#"fn main() { let s = "a" < "b" print(s) }"#);
    assert!(js.contains(r#"let s = ("a" < "b");"#));
}

#[test]
fn match_enum_payload_destructures_each_slot() {
    let js = generate_js(
        r#"enum Pair { A(number, string) B }
        fn main() { let p = Pair::A(1, "x") let m = match p { Pair::A(a, b) => "ok" Pair::B => "no" } print(m) }"#,
    );
    assert!(js.contains("tag === \"A\""));
    assert!(js.contains("const a = __m.value[0];"));
    assert!(js.contains("const b = __m.value[1];"));
}

#[test]
fn enum_multi_payload_constructor_emits_array_value() {
    let js = generate_js(
        r#"enum Pair<T> { A(T, string) B }
        fn main() { let p = Pair::A(7, "x") print(1) }"#,
    );
    assert!(js.contains("A: (p0, p1) => ({ tag: \"A\", value: [p0, p1] })"));
}

#[test]
fn state_reassignment_emits_setter() {
    let js = generate_js(
        r#"fn main(): Component { @State let n: number = 0 n = 4 VStack { Text(str(n)) } }"#,
    );
    assert!(js.contains("n.set(4);"));
    assert!(js.contains("const n = __signal(0);"));
}

#[test]
fn str_builtin_maps_to_string() {
    let js =
        generate_js(r#"fn main() { let s = str(3.5 + 1) let b = str(true) print(s) print(b) }"#);
    assert!(js.contains("String((3.5 + 1))"));
    assert!(js.contains("String(true)"));
}

#[test]
fn optional_member_emits_question_dot() {
    let js =
        generate_js(r#"fn main() { let u: { name: string }? = null let n = u?.name print(n) }"#);
    assert!(js.contains("let n = u?.name;"));
}

#[test]
fn optional_method_call_emits_question_dot() {
    let js =
        generate_js(r#"fn main() { let u: { name: string }? = null let n = u?.trim() print(n) }"#);
    assert!(js.contains("u?.trim()"));
}

#[test]
fn named_arguments_fill_defaulted_slots() {
    let js = generate_js(
        r#"fn greet(name: string, punct: string = "!"): string { name + punct }
        fn main() { print(greet(name: "world")) print(greet(name: "hi", punct: "?")) }"#,
    );
    assert!(js.contains("greet(\"world\", undefined)"), "js: {js}");
    assert!(js.contains("greet(\"hi\", \"?\")"), "js: {js}");
}

#[test]
fn objet_and_number_receivers_are_parenthesized() {
    let js = generate_js(r#"fn main() { let r = {}.a let n = 5.toString() print(r) print(n) }"#);
    assert!(js.contains("({}).a"), "js: {js}");
    assert!(js.contains("(5).toString()"), "js: {js}");
}

#[test]
fn user_str_shadow_emits_plain_call() {
    // A user-declared `str` compiles to a normal call, not `String(...)`.
    let js = generate_js(
        r#"fn str(s: string): string { "[[" + s + "]]" }
        fn main() { print(str("hi")) print(str("bye")) }"#,
    );
    assert!(js.contains("console.log(str(\"hi\"));"), "js: {js}");
    assert!(!js.contains("String(str"), "js: {js}");
}

#[test]
fn exported_function_parameter_names_registered() {
    // `export fn` params are registered so the built module can reorder named
    // arguments via the bundler.
    let js = generate_js(
        r#"export fn greet(name: string, punct: string): string { name + punct }
        fn main() { greet(punct: "!", name: "hi") }"#,
    );
    assert!(js.contains("greet(\"hi\", \"!\")"), "js: {js}");
}

#[test]
fn multi_statement_closure_keeps_shadowing_scope() {
    // A closure with several statement-local bindings must shadow an outer
    // `@State` inside its body (single child, not one per statement).
    let js = generate_js(
        r#"fn main(): Component {
            @State let name: string = "outer"
            @Effect fn() { let name: string = "inner" print(name) }
        }"#,
    );
    assert!(js.contains("let name = \"inner\""), "js: {js}");
}

#[test]
fn match_payload_binding_scopes_under_match() {
    let js = generate_js(
        r#"enum R<T> { A(T) B }
        fn main(): Component {
            let other = 1
            let v = R::A(9)
            match v { R::A(b) => print(b) R::B => print(0) }
        }"#,
    );
    assert!(js.contains("const b = __m.value"), "js: {js}");
}
