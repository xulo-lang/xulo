use std::path::Path;
use xulo_compiler::{codegen::CodeGen, compile_to_ir};

fn compile_and_run(source: &str) -> Vec<String> {
    let ir = compile_to_ir(source, Path::new("test.xulo")).expect("IR generation failed");
    let mut codegen = CodeGen::new();
    let code_ptr = codegen.compile(&ir).expect("codegen failed");

    unsafe {
        let func: fn() = std::mem::transmute(code_ptr);
        func();
    }
    xulo_runtime::runtime::xulo_take_output()
}

// -- RegExp::compile / test --

#[test]
fn compile_success_and_test() {
    let out = compile_and_run(
        r#"
use std::regex::{RegExp, RegexError};
fn main() {
    match RegExp::compile("[a-z]+", "i") {
        Result::Ok(re) => {
            print(re.test("ABC123"))
            print(re.test("123"))
        }
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out, vec!["true", "false"]);
}

#[test]
fn compile_bad_pattern_is_error_data() {
    let out = compile_and_run(
        r#"
use std::regex::{RegExp, RegexError};
fn main() {
    match RegExp::compile("[a-z") {
        Result::Ok(_) => print("compiled"),
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out.len(), 1);
    assert!(out[0].contains("unclosed character class"), "{}", out[0]);
}

#[test]
fn compile_unknown_flag_is_error_data() {
    let out = compile_and_run(
        r#"
use std::regex::{RegExp, RegexError};
fn main() {
    match RegExp::compile("a", "z") {
        Result::Ok(_) => print("compiled"),
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out.len(), 1);
    assert!(out[0].contains("unknown flag"), "{}", out[0]);
}

// -- find / find_all / group --

#[test]
fn find_match_value_and_groups() {
    let out = compile_and_run(
        r#"
use std::regex::{RegExp, RegexError};
fn main() {
    match RegExp::compile("(\\d+)-(\\d+)") {
        Result::Ok(re) => {
            let m = re.find("room 42-7")
            if m != null {
                print(m.value, m.group(1), m.group(2))
            } else {
                print("miss")
            }
        }
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out, vec!["42-7 42 7"]);
}

#[test]
fn find_miss_returns_null() {
    let out = compile_and_run(
        r#"
use std::regex::{RegExp, RegexError};
fn main() {
    match RegExp::compile("\\d+") {
        Result::Ok(re) => {
            let m = re.find("abc")
            if m != null {
                print("hit")
            } else {
                print("miss")
            }
        }
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out, vec!["miss"]);
}

#[test]
fn find_all_iterates_matches() {
    let out = compile_and_run(
        r#"
use std::regex::{RegExp, RegexError};
fn main() {
    match RegExp::compile("(\\d+)-(\\d+)") {
        Result::Ok(re) => {
            for hit in re.find_all("x 42-7 y 8-9") {
                print(hit.value)
            }
        }
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out, vec!["42-7", "8-9"]);
}

// -- replace: template / closure --

#[test]
fn replace_with_template() {
    let out = compile_and_run(
        r#"
use std::regex::{RegExp, RegexError};
fn main() {
    match RegExp::compile("(\\w+)@(\\w+)") {
        Result::Ok(re) => print("ada@example".replace(re, "$2.$1")),
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out, vec!["example.ada"]);
}

#[test]
fn replace_with_closure_all_matches() {
    let out = compile_and_run(
        r#"
use std::regex::{RegExp, RegexError};
fn main() {
    match RegExp::compile("oo", "g") {
        Result::Ok(re) => print("foo boo".replace(re, fn(m) { "[" + m.value + "]" })),
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out, vec!["f[oo] b[oo]"]);
}

// -- 字符串辅助方法 --

#[test]
fn string_to_uppercase() {
    let out = compile_and_run(r#"fn main() { print("abc".to_uppercase()) }"#);
    assert_eq!(out, vec!["ABC"]);
}

// -- `?` 传播 --

#[test]
fn propagate_ok_unwraps_payload() {
    let out = compile_and_run(
        r#"
use std::regex::{RegExp, RegexError};
fn compile_any(pattern: string): Result<RegExp, RegexError> {
    let re = RegExp::compile(pattern)?
    Result::Ok(re)
}
fn main() {
    match compile_any("[a-z]+") {
        Result::Ok(re) => print(re.test("az")),
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out, vec!["true"]);
}

#[test]
fn propagate_err_becomes_caller_result() {
    let out = compile_and_run(
        r#"
use std::regex::{RegExp, RegexError};
fn compile_any(pattern: string): Result<RegExp, RegexError> {
    let re = RegExp::compile(pattern)?
    Result::Ok(re)
}
fn main() {
    match compile_any("[a-z") {
        Result::Ok(_) => print("compiled"),
        Result::Err(e) => print("failed")
    }
}
"#,
    );
    assert_eq!(out, vec!["failed"]);
}

// -- match 臂块内语句依序执行（此前只求值末表达式） --

#[test]
fn match_arm_executes_preceding_statements() {
    let out = compile_and_run(
        r#"
fn main() {
    match Result::Ok(40) {
        Result::Ok(v) => {
            let doubled = v * 2
            print(doubled)
        }
        Result::Err(e) => print(e)
    }
}
"#,
    );
    assert_eq!(out, vec!["80"]);
}

// -- 无返回类型标注闭包：尾表达式作返回值 --

#[test]
fn untyped_closure_returns_tail_expression() {
    let out = compile_and_run(
        r#"
fn main() {
    let f = fn(x) { x + 1 }
    print(f(41))
}
"#,
    );
    assert_eq!(out, vec!["42"]);
}
