//! End-to-end macro expansion (spec `builtins/execution.md`,
//! `type-system/checking-rules.md`, "Expansion pass"): parse, expand, check,
//! run.

use xulo_core::error::XuloError;
use xulo_parser::parse_program;
use xulo_runtime::interpreter::Interpreter;
use xulo_runtime::macro_system::expand_program;

fn pipeline(src: &str) -> Result<Vec<String>, XuloError> {
    let mut program = parse_program(src).map_err(|e| e.with_message_prefix("parse: "))?;
    expand_program(&mut program, src, "test.xulo")?;
    xulo_semantic::analyze(&program)?;
    Interpreter::new().run(&program)
}

fn run_ok(src: &str) -> Vec<String> {
    pipeline(src).unwrap_or_else(|e| panic!("{}: {}", e.code(), e.message))
}

fn err_code(src: &str) -> &'static str {
    pipeline(src).expect_err("expected an error").code()
}

#[test]
fn function_macro_via_quote() {
    let out = run_ok(
        r#"
#[macro(kind = "function", name = "answer")]
fn answer_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  return Result::Ok(ast::FunctionOutput(expr: (ast::quote! { 40 + 2 })[0]))
}
fn main() {
  print(answer!())
}
"#,
    );
    assert_eq!(out, ["42"]);
}

#[test]
fn function_macro_via_ast_node() {
    let out = run_ok(
        r#"
#[macro(kind = "function", name = "hello")]
fn hello_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  return Result::Ok(ast::FunctionOutput(expr: ast::node("StringLiteral", { text: ast::Value::String("hi") })))
}
fn main() {
  print(hello!())
}
"#,
    );
    assert_eq!(out, ["hi"]);
}

#[test]
fn derive_macro_generates_an_impl() {
    let out = run_ok(
        r#"
trait Model { fn model(self): int }

#[macro(kind = "derive", target = Model)]
fn derive_model(input: ast::DeriveInput): Result<ast::DeriveOutput, ast::MacroFailure> {
  return Result::Ok(ast::DeriveOutput(items: ast::quote! {
    impl Model for ${input.target.name} {
      fn model(self): int { 7 }
    }
  }))
}

#[derive(Model)]
struct Thing { x: int }

fn main() {
  print(Thing(x: 1).model())
}
"#,
    );
    assert_eq!(out, ["7"]);
}

#[test]
fn attribute_macro_adds_an_item() {
    let out = run_ok(
        r#"
#[macro(kind = "attribute", name = "adds")]
fn adds_macro(input: ast::AttributeInput): Result<ast::AttributeOutput, ast::MacroFailure> {
  return Result::Ok(ast::AttributeOutput(
    item: input.target.node,
    extras: ast::quote! { fn added(): int { 5 } }
  ))
}

#[adds(3)]
fn main() {
  print(added())
}
"#,
    );
    assert_eq!(out, ["5"]);
}

#[test]
fn derive_keeps_builtin_debug_and_runs_user_targets() {
    let out = run_ok(
        r#"
#[macro(kind = "derive", target = Model)]
fn derive_model(input: ast::DeriveInput): Result<ast::DeriveOutput, ast::MacroFailure> {
  return Result::Ok(ast::DeriveOutput(items: ast::quote! {
    impl Model for ${input.target.name} { fn model(self): int { 3 } }
  }))
}
trait Model { fn model(self): int }
#[derive(Debug, Model)]
struct S { a: int }
fn main() {
  print(S(a: 1).model())
  print(S(a: 1))
}
"#,
    );
    assert_eq!(out, ["3", "S(a: 1)"]);
}

#[test]
fn macro_reported_failure_is_e1008() {
    assert_eq!(
        err_code(
            r#"
#[macro(kind = "function", name = "no")]
fn no_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  return Result::Err(ast::MacroFailure(message: "nope", span: null))
}
fn main() { print(no!()) }
"#
        ),
        "E1008"
    );
}

#[test]
fn malformed_definition_is_e1010() {
    assert_eq!(
        err_code(
            r#"
#[macro(kind = "derive")]
fn bad(input: ast::DeriveInput): Result<ast::DeriveOutput, ast::MacroFailure> {
  return Result::Err(ast::MacroFailure(message: "x", span: null))
}
fn main() {}
"#
        ),
        "E1010"
    );
}

#[test]
fn missing_function_macro_is_e1005() {
    assert_eq!(
        err_code(
            r#"
fn main() { print(missing!()) }
"#
        ),
        "E1005"
    );
}

#[test]
fn wrong_kind_is_e1006() {
    assert_eq!(
        err_code(
            r#"
#[macro(kind = "function", name = "route")]
fn route_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  return Result::Ok(ast::FunctionOutput(expr: ast::node("NumberLiteral", { text: ast::Value::Int(1) })))
}
#[route("/v1")]
fn main() {}
"#
        ),
        "E1006"
    );
}

#[test]
fn unresolved_argument_name_is_e1009() {
    assert_eq!(
        err_code(
            r#"
#[macro(kind = "function", name = "sql")]
fn sql_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  return Result::Ok(ast::FunctionOutput(expr: ast::node("NumberLiteral", { text: ast::Value::Int(1) })))
}
fn main() { print(sql!(user_name)) }
"#
        ),
        "E1009"
    );
}

#[test]
fn recursive_macro_call_is_e1003() {
    assert_eq!(
        err_code(
            r#"
#[macro(kind = "function", name = "recur")]
fn recur_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  let out = ast::call_function("recur", input)?
  return out
}
fn main() { print(recur!()) }
"#
        ),
        "E1003"
    );
}

#[test]
fn sandbox_violation_is_e1007() {
    assert_eq!(
        err_code(
            r#"
#[macro(kind = "function", name = "leak")]
fn leak_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  let text = std::fs::read("secret")
  return Result::Err(ast::MacroFailure(message: "x", span: null))
}
fn main() { print(leak!()) }
"#
        ),
        "E1007"
    );
}

#[test]
fn execution_trap_is_e1012() {
    assert_eq!(
        err_code(
            r#"
#[macro(kind = "function", name = "trap")]
fn trap_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  let xs = [1]
  let y = xs[9]
  return Result::Ok(ast::FunctionOutput(expr: ast::node("NumberLiteral", { text: ast::Value::Int(y) })))
}
fn main() { print(trap!()) }
"#
        ),
        "E1012"
    );
}

#[test]
fn malformed_quote_is_e1011() {
    assert_eq!(
        err_code(
            r#"
#[macro(kind = "function", name = "bad")]
fn bad_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  return Result::Ok(ast::FunctionOutput(expr: (ast::quote! { let ${1} = 2 })[0]))
}
fn main() { print(bad!()) }
"#
        ),
        "E1011"
    );
}

#[test]
fn unconvertible_call_input_is_e1002() {
    assert_eq!(
        err_code(
            r#"
#[macro(kind = "derive", target = Model)]
fn derive_model(input: ast::DeriveInput): Result<ast::DeriveOutput, ast::MacroFailure> {
  let out = ast::call_derive("Model", ast::node("StringLiteral", { text: ast::Value::String("x") }))?
  return out
}
trait Model { fn model(self): int }
#[derive(Model)]
struct S { a: int }
fn main() {}
"#
        ),
        "E1002"
    );
}

#[test]
fn step_quota_is_e1001() {
    assert_eq!(
        err_code(
            r#"
#[macro(kind = "function", name = "spin", max_steps = 100)]
fn spin_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  let mut i = 0
  while true { i = i + 1 }
  return Result::Ok(ast::FunctionOutput(expr: ast::node("NumberLiteral", { text: ast::Value::Int(i) })))
}
fn main() { print(spin!()) }
"#
        ),
        "E1001"
    );
}

#[test]
fn implementation_function_is_runtime_testable() {
    let src = r#"
#[macro(kind = "function", name = "answer")]
fn answer_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  return Result::Ok(ast::FunctionOutput(expr: (ast::quote! { 1 })[0]))
}
#[test]
fn implementation_test() {
  let span = ast::Span(file: "t", start: 0, end: 0, line: 1, column: 1)
  let target = ast::ExpressionInfo(
    kind: "macro_call",
    text: "answer!()",
    node: ast::node("Expression", {}),
    span: span
  )
  let input = ast::FunctionInput(args: [], target: target)
  match answer_macro(input) {
    Result::Ok(_) => assert(true, "ok"),
    Result::Err(_) => assert(false, "unexpected failure")
  }
}
fn main() {}
"#;
    let mut program = parse_program(src).unwrap();
    expand_program(&mut program, src, "test.xulo").unwrap();
    xulo_semantic::analyze(&program).unwrap();
    let test = program
        .statements
        .iter()
        .find_map(|s| match s {
            xulo_core::ast::Statement::Test(f) => Some(f),
            _ => None,
        })
        .expect("test declaration");
    let interp = Interpreter::new();
    interp
        .run_test(&program, &[], test)
        .unwrap_or_else(|e| panic!("{}: {}", e.code(), e.message));
}

fn expand_error(src: &str) -> XuloError {
    let mut program = parse_program(src).unwrap();
    expand_program(&mut program, src, "test.xulo").unwrap_err()
}

#[test]
fn nested_calls_share_the_root_step_budget() {
    let src = r#"
#[macro(kind = "function", name = "inner", max_steps = 1000000)]
fn inner_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  let mut i = 0
  while i < 1000 { i = i + 1 }
  return Result::Ok(ast::FunctionOutput(expr: (ast::quote! { 1 })[0]))
}
#[macro(kind = "function", name = "outer", max_steps = 40)]
fn outer_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  let nested = ast::FunctionInput(args: [], target: input.target)
  match ast::call_function("inner", nested) {
    Result::Ok(out) => { return Result::Ok(out) },
    Result::Err(failure) => { return Result::Err(failure) }
  }
}
fn main() { let _ = outer!() }
"#;
    let err = expand_error(src);
    assert_eq!(err.code(), "E1001", "{}", err.message);
    assert!(err.message.contains("step"), "{}", err.message);
}

#[test]
fn call_depth_quota_is_e1001() {
    let src = r#"
fn descend(n: int) {
  if n > 0 { descend(n - 1) }
}
#[macro(kind = "function", name = "deep", max_depth = 2)]
fn deep_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  descend(5)
  return Result::Ok(ast::FunctionOutput(expr: (ast::quote! { 1 })[0]))
}
fn main() { let _ = deep!() }
"#;
    let err = expand_error(src);
    assert_eq!(err.code(), "E1001", "{}", err.message);
    assert!(err.message.contains("depth"), "{}", err.message);
}

#[test]
fn sandbox_applies_through_helpers() {
    let src = r#"
fn helper() { spawn async { } }
#[macro(kind = "function", name = "bad")]
fn bad_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  helper()
  return Result::Ok(ast::FunctionOutput(expr: (ast::quote! { 1 })[0]))
}
fn main() { let _ = bad!() }
"#;
    let err = expand_error(src);
    assert_eq!(err.code(), "E1007", "{}", err.message);
}

#[test]
fn memory_quota_is_e1001() {
    let src = r#"
#[macro(kind = "function", name = "big", max_memory = 8)]
fn big_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  let text = "0123456789abcdef"
  return Result::Ok(ast::FunctionOutput(expr: ast::node("StringLiteral", { text: ast::Value::String(text) })))
}
fn main() { print(big!()) }
"#;
    let err = expand_error(src);
    assert_eq!(err.code(), "E1001", "{}", err.message);
    assert!(err.message.contains("memory"), "{}", err.message);
}

#[test]
fn nested_calls_share_the_root_memory_budget() {
    let src = r#"
#[macro(kind = "function", name = "inner")]
fn inner_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  let text = "0123456789abcdef"
  return Result::Ok(ast::FunctionOutput(expr: ast::node("StringLiteral", { text: ast::Value::String(text) })))
}
#[macro(kind = "function", name = "outer", max_memory = 8)]
fn outer_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
  let nested = ast::FunctionInput(args: [], target: input.target)
  match ast::call_function("inner", nested) {
    Result::Ok(out) => { return Result::Ok(out) },
    Result::Err(failure) => { return Result::Err(failure) }
  }
}
fn main() { let _ = outer!() }
"#;
    let err = expand_error(src);
    assert_eq!(err.code(), "E1001", "{}", err.message);
    assert!(err.message.contains("memory"), "{}", err.message);
}

#[test]
fn generated_macro_definition_is_e1010() {
    let src = r#"
#[macro(kind = "attribute", name = "genmac")]
fn genmac_macro(input: ast::AttributeInput): Result<ast::AttributeOutput, ast::MacroFailure> {
  return Result::Ok(ast::AttributeOutput(
    item: (ast::quote! {
      #[macro(kind = "function", name = "late")]
      fn late_macro(input: ast::FunctionInput): Result<ast::FunctionOutput, ast::MacroFailure> {
        return Result::Ok(ast::FunctionOutput(expr: (ast::quote! { 1 })[0]))
      }
    })[0],
    extras: []
  ))
}
#[genmac]
fn target() {}
fn main() {}
"#;
    let err = expand_error(src);
    assert_eq!(err.code(), "E1010", "{}", err.message);
}
