use xulo::codegen::generate;
use xulo::lexer::tokenize;
use xulo::parser::parse_program;
use xulo::semantic::analyze;

fn generate_js(src: &str) -> String {
    let tokens = tokenize(src).unwrap();
    let program = parse_program(&tokens).unwrap();
    analyze(&program).unwrap();
    generate(&program).unwrap()
}

#[test]
fn function_and_call() {
    let js = generate_js(r#"fn add(a: number, b: number): number { return a + b } fn main() { print(add(1, 2)) }"#);
    assert!(js.contains("function add(a, b) {"));
    assert!(js.contains("return (a + b);"));
    assert!(js.contains("console.log(add(1, 2));"));
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
    assert!(js.contains("for (const item of [1, 2]) {"));
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
    let js = generate_js("fn main() { let u: { name: string }? = null print(u?.name ?? \"anon\") }");
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
    let js = generate_js("fn main() { let double = fn(x: number): number { x * 2 } print(double(3)) }");
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
