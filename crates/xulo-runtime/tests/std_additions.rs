//! The first expansion batch of the standard library: `std::math` signs and
//! helpers, `std::path` inspection, `std::os` host directories/identity,
//! `std::io` terminal probes, `std::json` validation/indentation, and
//! `std::regex` positions/named groups/dotall/split.

use xulo_core::error::XuloError;
use xulo_parser::parse_program;

fn run(src: &str) -> Result<Vec<String>, XuloError> {
    let program = parse_program(src).unwrap();
    xulo_semantic::analyze(&program)?;
    let interp = xulo_runtime::interpreter::Interpreter::new();
    interp.run(&program)
}

fn run_ok(src: &str) -> Vec<String> {
    run(src).unwrap_or_else(|e| panic!("{}", e.message))
}

#[test]
fn math_helpers() {
    assert_eq!(
        run_ok(
            r#"
            fn main() {
                print(std::math::sign(-3.5))
                print(std::math::sign(0))
                print(std::math::hypot(3, 4))
                print(std::math::clamp(12, 0, 10))
                print(std::math::log1p(0))
                print(std::math::expm1(0))
                let r = std::math::random_int(1, 6)
                print(r >= 1 and r <= 6)
            }
            "#
        ),
        vec!["-1", "0", "5", "10", "0", "0", "true"]
    );
}

#[test]
fn math_random_int_empty_range_is_a_runtime_failure() {
    let err = run(r#"fn main() { std::math::random_int(6, 1) }"#).unwrap_err();
    assert!(err.message.contains("empty"), "{}", err.message);
}

#[test]
fn path_inspection_members() {
    assert_eq!(
        run_ok(
            r#"
            fn main() {
                print(std::path::is_absolute("/a"))
                print(std::path::is_absolute("a"))
                let s = std::path::split("/var/log/app.log")
                print(s.dir)
                print(s.base)
                print(s.ext)
                print(std::path::stem("archive.tar.gz"))
                print(std::path::with_ext("report.txt", ".md"))
                print(std::path::with_ext("report.txt", ""))
                print(std::path::relative("/var/log", "/etc"))
                print(std::path::relative("a/b", "a/c"))
                print(std::path::relative("/a", "b"))
            }
            "#
        ),
        vec![
            "true",
            "false",
            "/var/log",
            "app.log",
            ".log",
            "archive.tar",
            "report.md",
            "report",
            "../../etc",
            "../c",
            "null",
        ]
    );
}

#[test]
fn os_host_surface() {
    assert_eq!(
        run_ok(
            r#"
            fn main() {
                print(std::os::tmp_dir().len > 0)
                print(std::os::cpu_count() >= 1)
                match std::os::home_dir() {
                    Result::Ok(h) => print(h.len > 0)
                    Result::Err(e) => print(e.message)
                }
                std::os::setenv("XULO_TEST_VAR", "1")
                match std::os::getenv("XULO_TEST_VAR") {
                    Result::Ok(v) => print(v)
                    Result::Err(e) => print(e.message)
                }
                std::os::unsetenv("XULO_TEST_VAR")
                match std::os::getenv("XULO_TEST_VAR") {
                    Result::Ok(v) => print(v == null)
                    Result::Err(e) => print(e.message)
                }
            }
            "#
        ),
        vec!["true", "true", "true", "1", "true"]
    );
}

#[test]
fn io_terminal_probes_are_bools() {
    let out =
        run_ok(r#"fn main() { print(std::io::stdin_is_tty()); print(std::io::stdout_is_tty()) }"#);
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|v| v == "true" || v == "false"), "{out:?}");
}

#[test]
fn json_valid_and_indent() {
    assert_eq!(
        run_ok(
            r#"
            fn main() {
                print(std::json::valid("{\"a\": 1}"))
                print(std::json::valid("{\"a\": }"))
                match std::json::stringify({ n: 3 }, 2) {
                    Result::Ok(t) => print(t)
                    Result::Err(e) => print(e.message)
                }
            }
            "#
        ),
        vec!["true", "false", "{\n  \"n\": 3\n}"]
    );
}

#[test]
fn regex_positions_named_groups_dotall_and_split() {
    assert_eq!(
        run_ok(
            r#"
            use std::regex::{RegExp};
            fn main() {
                match RegExp::compile("(?P<y>\\d+)-(?P<m>\\d+)", "") {
                    Result::Ok(re) => {
                        let m = re.find("date 2026-10")
                        if m != null {
                            print(m.value)
                            print(m.index)
                            print(m.end)
                            print(m.group("y"))
                        }
                    }
                    Result::Err(e) => print(e.message)
                }
                match RegExp::compile(",", "") {
                    Result::Ok(re) => print("a,b,".split(re))
                    Result::Err(e) => print(e.message)
                }
                match RegExp::compile("a.b", "s") {
                    Result::Ok(re) => print(re.test("a\nb"))
                    Result::Err(e) => print(e.message)
                }
            }
            "#
        ),
        vec!["2026-10", "5", "12", "2026", "[a, b, ]", "true"]
    );
}
