use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_xulo");

fn temp_file(name: &str, content: &str) -> PathBuf {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "xulo_it_{}_{}_{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path
}

/// Create a throwaway directory holding `files` (name -> content) and return
/// its path plus the path of `entry`.
fn temp_dir(files: &[(&str, &str)], entry: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "xulo_md_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, content) in files {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }
    (dir.clone(), dir.join(entry))
}

#[test]
fn run_hello_world() {
    let file = temp_file("hello.xulo", r#"fn main() { print("Hello, world!") }"#);
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "Hello, world!\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_arithmetic() {
    let file = temp_file(
        "arith.xulo",
        "fn main() { let a = 2 let b = 3 print(a * b) }",
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "6\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_overflowing_number_literal_infinity() {
    // A 310-digit literal overflows f64 to +Infinity. The JS path formats it as
    // the `Infinity` global (Rust's "inf" is not valid JS and used to throw a
    // ReferenceError under node).
    let huge = format!("1{}", "0".repeat(309));
    let file = temp_file("inf.xulo", &format!("fn main() {{ print({huge}) }}"));
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "Infinity\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_fibonacci_recursion() {
    let file = temp_file(
        "fib.xulo",
        r#"
        fn fib(n: number): number {
            if n <= 1 { return n }
            else { return fib(n - 1) + fib(n - 2) }
        }
        fn main() { print(fib(8)) }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "21\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn check_reports_errors() {
    let file = temp_file("bad.xulo", "fn main() { print(undefined_name) }");
    let out = Command::new(BIN).arg("check").arg(&file).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("undefined variable `undefined_name`"));
    let _ = std::fs::remove_file(&file);
}

#[test]
fn fmt_formats_and_repl_runs() {
    let file = temp_file("f.xulo", "fn main(){print(1)}");
    let fmt = Command::new(BIN).arg("fmt").arg(&file).output().unwrap();
    assert!(fmt.status.success());
    let formatted = std::fs::read_to_string(&file).unwrap();
    assert!(formatted.contains("fn main() {"));
    assert!(formatted.contains("print(1)"));

    let repl = Command::new(BIN)
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = repl.stdin.as_ref().unwrap();
    stdin.write_all(b"print(21 + 21)\n\nexit\n").unwrap();
    let out = repl.wait_with_output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("42"));

    let _ = std::fs::remove_file(&file);
}

#[test]
fn repl_assignment_and_run() {
    // Assignments must compile as statements (not be mis-echoed as an
    // expression, which previously produced a spurious error); single lines
    // execute immediately, and `run` forces the whole session to re-run.
    let repl = Command::new(BIN)
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = repl.stdin.as_ref().unwrap();
    stdin
        .write_all(b"let mut x = 5\nx = x + 2\nprint(x)\nrun\nexit\n")
        .unwrap();
    let out = repl.wait_with_output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    // `print(x)` evaluates x to 7; `run` re-runs the session and prints it again.
    assert!(
        text.matches('7').count() >= 2,
        "missing assignment result in:\n{text}"
    );
}

#[test]
fn repl_single_line_expression_echoes_immediately() {
    // A complete single-line entry (here a comparison) must echo on Enter,
    // not wait for a second Enter.
    let repl = Command::new(BIN)
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = repl.stdin.as_ref().unwrap();
    stdin.write_all(b"4>5\nexit\n").unwrap();
    let out = repl.wait_with_output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("false"), "missing echo in:\n{text}");
}

#[test]
fn default_no_args_enters_repl() {
    // `xulo` with no arguments (and no subcommand) starts the REPL.
    let repl = Command::new(BIN)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = repl.stdin.as_ref().unwrap();
    stdin.write_all(b"print(21 + 21)\n\nexit\n").unwrap();
    let out = repl.wait_with_output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Welcome to xulo"));
    assert!(text.contains("42"));
}

#[test]
fn repl_echo_colors_values() {
    // Echoed values are colorized by runtime type like `node`'s `util.inspect`
    // (strings green, numbers/booleans yellow, null grey). `CLICOLOR_FORCE`
    // makes colors visible even though the test stdout is a pipe.
    let repl = Command::new(BIN)
        .arg("repl")
        .env("CLICOLOR_FORCE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = repl.stdin.as_ref().unwrap();
    stdin.write_all(b"4>5\n\"hi\"\nnull\n123\nexit\n").unwrap();
    let out = repl.wait_with_output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("\x1b[33mfalse\x1b[39m\x1b[22m"),
        "missing yellow boolean in:\n{text}"
    );
    assert!(
        text.contains("\x1b[32mhi\x1b[39m\x1b[22m"),
        "missing green string in:\n{text}"
    );
    assert!(
        text.contains("\x1b[1m\x1b[90mnull\x1b[39m\x1b[22m"),
        "missing grey null in:\n{text}"
    );
    assert!(
        text.contains("\x1b[33m123\x1b[39m\x1b[22m"),
        "missing yellow number in:\n{text}"
    );
}

#[test]
fn run_types_enums_const_null() {
    let file = temp_file(
        "types.xulo",
        r#"
        enum Theme { Light Dark }
        enum Result<T> { Success(T) Error(string) }
        type Status = "active" | "inactive"

        const APP = "Xulo"
        let mut count = 1

        fn run(): number {
          count = count + 1
          count
        }
        fn main() {
          let theme = Theme::Dark
          let ok = Result::Success(42)
          let s: string? = null
          print(theme == Theme::Dark)
          print(ok)
          print(APP)
          print(count)
          print(run())
          print(s == null)
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "true\nResult.Success(42)\nXulo\n1\n2\ntrue\n"
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_multi_payload_enum() {
    let file = temp_file(
        "multipayload.xulo",
        r#"
        enum Person { Nobody, Named(string, number) }

        fn greet(p: Person): string {
            match p {
                Person::Named(name, age) => "hi " + name + " (" + str(age) + ")"
                Person::Nobody => "hi anon"
            }
        }

        fn main() {
            print(greet(Person::Named("Ada", 36)))
            print(greet(Person::Nobody))
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "hi Ada (36)\nhi anon\n"
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_phase2_control_flow_and_expressions() {
    let file = temp_file(
        "phase2.xulo",
        r#"
        enum Result<T> { Success(T) Error(string) }

        fn describe(r: Result<number>): string {
          match r {
            Result::Success(v) => "got " + v
            Result::Error(msg) => "err: " + msg
          }
        }

        fn main() {
          let mut total = 0
          for i in 0..<5 { total = total + i }
          print(total)

          let mut c = 0
          while c < 3 { c = c + 1 }
          print(c)

          let name: string? = null
          print(name ?? "anon")

          let user = { name: "Xulo" }
          print(user.name)

          let obj = { a: 1 }
          let merged = { ...obj, b: 2 }
          print(merged.b)

          print(false or true and true)
          print(!false)

          let flag = 1 > 2 ? "no" : "yes"
          print(flag)

          print(match 9 { 0 => "zero" _ => "other" })

          print(describe(Result::Success(7)))
          print(describe(Result::Error("boom")))
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "10\n3\nanon\nXulo\n2\ntrue\ntrue\nyes\nother\ngot 7\nerr: boom\n"
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_closed_range_loops() {
    // `a...b` is a Swift-style closed range (inclusive upper bound), while
    // `a..<b` stays half-open. Both drive `for` iteration and evaluate to a
    // list of numbers when used as an expression.
    let file = temp_file(
        "range.xulo",
        r#"
        fn main() {
            for i in 0...2 { print(i) }
            for i in 3..<6 { print(i) }
            let closed = 0...3
            let half = 0..<3
            print(closed)
            print(half)
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "0\n1\n2\n3\n4\n5\n[0, 1, 2, 3]\n[0, 1, 2]\n"
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_multi_file_imports() {
    let (dir, entry) = temp_dir(
        &[
            (
                "math.xulo",
                r#"
                pub fn add(a: number, b: number): number { return a + b }
                pub const PI = 3.14
                "#,
            ),
            (
                "main.xulo",
                r#"
                mod math;
                use math::{add, PI};
                fn main() {
                    print(add(2, 3))
                    print(PI)
                    print(math::add(10, 1))
                }
                "#,
            ),
        ],
        "main.xulo",
    );
    let out = Command::new(BIN).arg("run").arg(&entry).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "5\n3.14\n11\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_multi_file_named_and_async() {
    let (dir, entry) = temp_dir(
        &[
            (
                "greet.xulo",
                r#"
                pub fn greet(name: string): string { return "hi " + name }
                "#,
            ),
            (
                "load.xulo",
                r#"
                pub async fn load(n: number): number {
                    let x = n * 2
                    return x
                }
                "#,
            ),
            (
                "main.xulo",
                r#"
                mod greet;
                mod load;
                use greet::greet;
                use load::load;
                async fn main() {
                    print(greet("bob"))
                    let v = await load(21)
                    print(v)
                }
                "#,
            ),
        ],
        "main.xulo",
    );
    let out = Command::new(BIN).arg("run").arg(&entry).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hi bob\n42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_pub_module_exports() {
    let (dir, entry) = temp_dir(
        &[
            (
                "math.xulo",
                "pub fn add(a: number, b: number): number { return a + b }\n\
                 pub const PI = 3.14\n\
                 pub enum Role { Admin Guest }\n\
                 fn secret() { print(\"no\") }\n",
            ),
            (
                "main.xulo",
                "mod math;\n\
                 use math::{add, PI, Role};\n\
                 fn main() { print(add(2, 3)) print(PI) print(Role::Admin) }\n",
            ),
        ],
        "main.xulo",
    );
    let out = Command::new(BIN).arg("run").arg(&entry).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "5\n3.14\nRole.Admin\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_pub_main_entry() {
    let file = temp_file(
        "pubmain.xulo",
        "pub fn main() { print(\"hi from pub main\") }",
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hi from pub main\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn rejects_reserved_word_programs() {
    let file = temp_file("reserved.xulo", "fn main() { let class = 1 }");
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("reserved keyword")
            || stderr.contains("unexpected")
            || stderr.contains("expected"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn allows_stdlib_type_names_as_identifiers() {
    // The composite/prelude type names are recommended against, but not
    // reserved: code that uses them as identifiers must keep working. The
    // primitive type names *are* keywords now (spec
    // `lexical-structure.md`), and so are reserved.
    let file = temp_file(
        "typenames.xulo",
        "fn main() {\n\
             let list = [1]\n\
             let view = \"v\"\n\
             let error = 42\n\
             print(list) print(view) print(error)\n\
         }",
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "[1]\nv\n42\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_module_import_destructure() {
    let (dir, entry) = temp_dir(
        &[
            (
                "util.xulo",
                "pub fn double(x: number): number { return x * 2 }\n",
            ),
            (
                "main.xulo",
                "mod util;\nuse util::double;\nfn main() { print(double(4)) }\n",
            ),
        ],
        "main.xulo",
    );
    let out = Command::new(BIN).arg("run").arg(&entry).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "8\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_reports_missing_export() {
    let (dir, entry) = temp_dir(
        &[
            (
                "util.xulo",
                "pub fn double(x: number): number { return x * 2 }\n",
            ),
            (
                "main.xulo",
                "mod util;\nuse util::triple;\nfn main() { print(triple(1)) }\n",
            ),
        ],
        "main.xulo",
    );
    let out = Command::new(BIN).arg("check").arg(&entry).output().unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("does not export `triple`"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_accepts_circular_import() {
    // `use` cycles are legal (the interface phase is two-pass): a and b may
    // read each other's exports, and checking succeeds.
    let (dir, entry) = temp_dir(
        &[
            ("a.xulo", "use b::b;\npub fn a(): number { return b() }\n"),
            ("b.xulo", "use a::a;\npub fn b(): number { return a() }\n"),
            (
                "main.xulo",
                "pub mod a;\npub mod b;\nuse a::a;\nfn main() { print(a()) }\n",
            ),
        ],
        "main.xulo",
    );
    let out = Command::new(BIN).arg("check").arg(&entry).output().unwrap();
    assert!(
        out.status.success(),
        "cycles must not be rejected: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn type_only_enum_import_still_binds_the_value() {
    // The enum's *type* is imported `use type`; its runtime value comes from
    // the executing module (enums register by name in the interpreter), so
    // construction keeps working with no value binding.
    let (dir, entry) = temp_dir(
        &[
            ("colors.xulo", "pub enum Kind { User Admin }\n"),
            (
                "main.xulo",
                r#"
                mod colors;
                use type colors::Kind;
                fn main() { print(Kind::Admin) }
                "#,
            ),
        ],
        "main.xulo",
    );
    let out = Command::new(BIN).arg("run").arg(&entry).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "Kind.Admin\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_trait_dispatch_native() {
    let src = r#"
        trait Area { fn area(self): number; fn perimeter(self): number }
        struct Rectangle { w: number, h: number }
        impl Area for Rectangle {
            fn area(self): number { return self.w * self.h }
            fn perimeter(self): number { return 2 * (self.w + self.h) }
        }
        fn rect(w: number, h: number): Rectangle {
            return Rectangle(w: w, h: h)
        }
        fn main() {
            print("area=" + str(Area::area(rect(3, 4))))
            print("perimeter=" + str(Area::perimeter(rect(3, 4))))
        }
    "#;
    let file = temp_file("trait.xulo", src);
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "area=12\nperimeter=14\n",
        ""
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_list_concat_native() {
    // `list + list` concatenates the lists (native runtime).
    let src = r#"
        fn main() {
            let a = [1, 2]
            let b = [3, 4]
            print(a + b)
            print([0] + [7])
        }
    "#;
    let file = temp_file("listcat.xulo", src);
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Lists render as `[1, 2, 3, 4]`; `str()` on a list is E0212 (spec
    // `builtins/intrinsic-functions.md`), so the second line prints the
    // concatenation directly.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout, "[1, 2, 3, 4]\n[0, 7]\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_builtin_named_arguments_native() {
    // Named arguments on the variadic builtins `print`/`str` are allowed and
    // rendered in source order by the native runtime.
    let src = r#"
        fn main() {
            print(msg: "hi")
            print(1, b: 2, c: 3)
            print(str(value: 42))
        }
    "#;
    let file = temp_file("builtin_named.xulo", src);
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hi\n1 2 3\n42\n", "");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_loop_closure_capture_native() {
    // An `fn` declared in a loop body captures *its own iteration* of the loop
    // variable, not a shared binding that reads the final value from every
    // closure (native `exec_for` re-creates the iteration Env).
    let src = r#"
        fn main() {
            let mut funcs = []
            for i in 0..<3 {
                fn get(): number { i }
                funcs = [...funcs, get]
            }
            for j in 0..<3 {
                print(funcs[j]())
            }
        }
    "#;
    let file = temp_file("loopcap.xulo", src);
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "0\n1\n2\n",
        "each closure reads its own iteration of `i`"
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_for_loop_var_reassignment_native() {
    // `x = x + 1` inside `for x in xs` is legal (the loop variable is
    // mutable): the source list is left untouched.
    let src = r#"
        fn main() {
            let xs = [1, 2, 3]
            for x in xs {
                x = x + 1
            }
            print(xs)
        }
    "#;
    let file = temp_file("forvar.xulo", src);
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout, "[1, 2, 3]\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_enum_equality_native() {
    // Payload enums compare structurally: separately-constructed `Result::Ok(1)`
    // values are equal, and differ from `Result::Ok(2)`.
    let src = r#"
        enum Result { Ok(number) Err(string) }
        fn main() {
            let a = Result::Ok(1)
            let b = Result::Ok(1)
            let c = Result::Ok(2)
            print(str(a == b))
            print(str(a == c))
            print(str(a != c))
        }
    "#;
    let file = temp_file("enumeq.xulo", src);
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "true\nfalse\ntrue\n",
        ""
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_plain_tagged_objects_keep_identity_equality() {
    // A plain object that happens to have a `tag` key is *not* an enum: it
    // compares by identity, while enum values still compare structurally.
    let src = r#"
        enum R { Ok(number) }
        fn main() {
            let a = { tag: "Ok", value: 1 }
            let b = { tag: "Ok", value: 1 }
            print(str(a == b))
            // Enum values still compare structurally.
            let e1 = R::Ok(1)
            let e2 = R::Ok(1)
            print(str(e1 == e2))
        }
    "#;
    let file = temp_file("tagobj.xulo", src);
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "false\ntrue\n", "");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn cross_module_trait_dispatch() {
    // The `impl` is declared in a library module; the dependent entry module
    // imports the receiver type and dispatches `Shape::area` on it across
    // module boundaries.
    let (dir, entry) = temp_dir(
        &[
            (
                "shapes.xulo",
                r#"
                pub trait Shape { fn area(self): number }
                pub struct Rect { w: number, h: number }
                impl Shape for Rect {
                    fn area(self): number { return self.w * self.h }
                }
                pub fn rect(w: number, h: number): Rect {
                    return Rect(w: w, h: h)
                }
                "#,
            ),
            (
                "main.xulo",
                r#"
                mod shapes;
                use type shapes::Shape;
                use type shapes::Rect;
                use shapes::rect;
                fn main() {
                    print("area=" + str(Shape::area(rect(3, 4))))
                }
                "#,
            ),
        ],
        "main.xulo",
    );
    let out = Command::new(BIN).arg("run").arg(&entry).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "area=12\n", "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn imported_function_supports_named_arguments() {
    let (dir, entry) = temp_dir(
        &[
            (
                "util.xulo",
                "pub fn greet(name: string, times: number): string { name }\n",
            ),
            (
                "main.xulo",
                "mod util;\nuse util::greet;\nfn main() { print(greet(times: 2, name: \"hi\")) }\n",
            ),
        ],
        "main.xulo",
    );
    let out = Command::new(BIN).arg("run").arg(&entry).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hi\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_closures_and_higher_order_functions() {
    let file = temp_file(
        "closure.xulo",
        r#"
        fn apply(f: fn(number): number, x: number): number { return f(x) }
        fn makeAdder(n: number): fn(number): number {
            return fn(v: number): number { v + n }
        }
        fn main() {
            let double = fn(x: number): number { x * 2 }
            print(apply(double, 4))
            let add5 = makeAdder(5)
            print(add5(10))
            let mut count = 0
            let bump = fn() { count = count + 1 }
            bump()
            bump()
            print(count)
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "8\n15\n2\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_async_closure() {
    let file = temp_file(
        "ac.xulo",
        r#"
        async fn main() {
            let work = async fn() { 21 * 2 }
            print(await work())
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "42\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_list_spread() {
    let file = temp_file(
        "spread.xulo",
        r#"
        fn main() {
            let head = [1, 2]
            let tail = [3, 4]
            let all = [...head, ...tail]
            let mut sum = 0
            for x in all { sum = sum + x }
            print(sum)
            let merged = [...tail, ...head]
            print(merged[0])
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "10\n3\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_list_spread_with_computed_values() {
    let file = temp_file(
        "spread2.xulo",
        r#"
        fn build(xs: List<number>, extra: number): List<number> {
            return [...xs, extra]
        }
        fn main() {
            let r = build([1, 2], 9)
            print(r[2])
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "9\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_comma_separated_match() {
    let file = temp_file(
        "matchcomma.xulo",
        r#"
        enum Color { Red, Green, Blue }
        fn name(c: Color): string {
            match c {
                Color::Red => "red",
                Color::Green => "green",
                Color::Blue => "blue",
            }
        }
        fn main() {
            print(name(Color::Green))
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "green\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_comma_separated_match_with_enum_payload() {
    let file = temp_file(
        "matchpayload.xulo",
        r#"
        enum Maybe { Some(number), None }
        fn unwrap(m: Maybe): number {
            match m {
                Maybe::Some(v) => v,
                Maybe::None => 0,
            }
        }
        fn main() {
            print(unwrap(Maybe::Some(99)))
            print(unwrap(Maybe::None))
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "99\n0\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_function_values_from_expressions() {
    let file = temp_file(
        "callval.xulo",
        r#"
        fn makeAdder(n: number): fn(number): number {
          fn(v: number): number { v + n }
        }
        fn main() {
          let ops = [fn(a: number, b: number): number { a + b }, fn(a: number, b: number): number { a * b }]
          print(ops[0](3, 4))
          print(ops[1](3, 4))
          let get = fn(): fn(number): number { fn(x: number): number { x + 100 } }
          print(get()(1))
          let add5 = makeAdder(5)
          print(add5(10))
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "7\n12\n101\n15\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_named_enum_payload() {
    let file = temp_file(
        "namedpayload.xulo",
        r#"
        enum Action { Click, Submit(data: Map<string, unknown>), Cancel }
        fn describe(a: Action): string {
          match a {
            Action::Click => "click",
            Action::Submit(d) => "submit",
            Action::Cancel => "cancel",
          }
        }
        fn main() {
          print(describe(Action::Submit({ x: 1 })))
          print(describe(Action::Click))
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "submit\nclick\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_optional_and_default_params() {
    let file = temp_file(
        "optparams.xulo",
        r#"
        fn f(a: number, b: string?, c: bool = true): string {
            let flag = if c { "yes" } else { "no" }
            if b == null { str(a) + ":" + flag } else { str(a) + ":" + str(b) + ":" + flag }
        }
        fn main() {
            print(f(1))
            print(f(2, "x"))
            print(f(3, "y", false))
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "1:yes\n2:x:yes\n3:y:no\n"
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_optional_chaining_on_null() {
    let file = temp_file(
        "optchain.xulo",
        r#"
        fn main() {
            let nobody = null
            print(nobody?.name ?? "anonymous")
            let user = { name: "Ann" }
            print(user?.name ?? "anonymous")
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "anonymous\nAnn\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_component_state_and_effect() {
    let file = temp_file(
        "comp.xulo",
        r#"
        fn main(): View {
            @State let count: number = 0
            @Effect fn() { print("mounted") }
            count = count + 1
            print("count=" + str(count))
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "mounted\ncount=1\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn check_rejects_decorator_outside_component() {
    let file = temp_file("bad.xulo", "fn main() { @State let count: number = 0 }");
    let out = Command::new(BIN).arg("check").arg(&file).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("returning `View`"));
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_component_loop_var_shadowing() {
    let file = temp_file(
        "shadow.xulo",
        r#"
        fn main(): View {
            @State let x: number = 0
            let ys = [1, 2, 3]
            for x in ys { print(x) }
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "1\n2\n3\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn check_rejects_effect_capturing_render_local() {
    let file = temp_file(
        "bad.xulo",
        r#"
        fn main(): View {
            let a = 5
            @Effect fn() { print(a) }
        }
        "#,
    );
    let out = Command::new(BIN).arg("check").arg(&file).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("@Effect"));
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_negative_and_float_arithmetic() {
    let file = temp_file(
        "neg.xulo",
        r#"
        fn main() {
            print(-5 * 3.5)
            print(10 / 4)
            print(-10 - -3)
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "-17.5\n2.5\n-7\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_string_escapes_output() {
    let file = temp_file(
        "esc.xulo",
        r#"
        fn main() {
            print("a\nb")
            print("tab\there")
            print("\u{1F600}")
            print('single\'quote')
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "a\nb\ntab\there\n😀\nsingle'quote\n"
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_member_and_index_assignment() {
    let file = temp_file(
        "mut.xulo",
        r#"
        fn main() {
            let mut xs: List<number> = [1, 2, 3]
            xs[0] = 10
            xs[2] = 30
            let mut user = { age: 20 }
            user.age = user.age + 5
            print(xs[0] + xs[1] + user.age)
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "37\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_str_builtin_conversions() {
    let file = temp_file(
        "strb.xulo",
        r#"
        fn main() {
            print("v=" + str(3.5))
            print(str(true))
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(out.status.success());
    // `str(null)` is E0212 — null has no string form (spec
    // `builtins/intrinsic-functions.md`); covered in the semantic suite.
    assert_eq!(String::from_utf8_lossy(&out.stdout), "v=3.5\ntrue\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_nested_loops_and_ranges() {
    let file = temp_file(
        "loops.xulo",
        r#"
        fn main() {
            let mut total = 0
            for i in 0..<3 {
                for j in 0..<2 {
                    total = total + (i + j)
                }
            }
            print(total)
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "9\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_else_if_chain_and_while() {
    let file = temp_file(
        "elseif.xulo",
        r#"
        fn classify(n: number): string {
            if n < 0 { "neg" }
            else if n == 0 { "zero" }
            else if n < 10 { "small" }
            else { "big" }
        }
        fn main() {
            let mut n = 0
            while n < 3 { print(classify(n - 1)) n = n + 1 }
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "neg\nzero\nsmall\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_nullish_ternary_and_match_on_strings() {
    let file = temp_file(
        "mix.xulo",
        r#"
        fn grade(s: string): string { match s { "A" => "excellent" _ => "ok" } }
        fn main() {
            let name = null ?? "anon"
            let g = 5 > 3 ? grade("A") : "?"
            print(name)
            print(g)
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "anon\nexcellent\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_generic_enum_and_payload_match() {
    let file = temp_file(
        "gen.xulo",
        r#"
        enum Box<T> { Put(T) Empty }
        enum Shape { Circle(number) Rect(number, number) }
        fn id<T>(x: T): T { x }
        fn unbox(b: Box<number>): number {
            match b { Box::Put(v) => v * 2 Box::Empty => 0 }
        }
        fn area(s: Shape): number {
            match s {
                Shape::Circle(r) => 3 * r * r
                Shape::Rect(w, h) => w * h
            }
        }
        fn main() {
            let boxed = Box::Put(id(7))
            let s = area(Shape::Rect(3, 4))
            let c = area(Shape::Circle(2))
            print(unbox(boxed))
            print(s)
            print(c)
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "14\n12\n12\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_typed_object_read_and_assign() {
    let file = temp_file(
        "typed.xulo",
        r#"
        fn main() {
            let mut people = [{ name: "a", age: 1 }, { name: "b", age: 2 }]
            people[1].age = people[1].age + 1
            print(people[0].name)
            print(people[1].age)
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "a\n3\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_optional_chaining_deep_on_null() {
    let file = temp_file(
        "opt.xulo",
        r#"
        fn main() {
            let u: Map<string, unknown>? = null
            print(u?.a?.b)
            print(u == null)
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "null\ntrue\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_multi_payload_enum_value_shape() {
    let file = temp_file(
        "multi.xulo",
        r#"
        enum Pair { A(number, string) B }
        fn main() {
            let p = Pair::A(1, "x")
            let m = match p { Pair::A(a, b) => "a" + str(a) + b Pair::B => "z" }
            print(p)
            print(m)
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "Pair.A([1, x])\na1x\n"
    );
    let _ = std::fs::remove_file(&file);
}

// ---------------------------------------------------------------------------
// Native runtime (`xulo run --native`): module imports, external rejection,
// and async/await orchestration at the CLI level.
// ---------------------------------------------------------------------------

/// Run `xulo run --native <entry>` in a throwaway dir and return its output.
fn native_run(files: &[(&str, &str)], entry: &str) -> (bool, String, String) {
    let (dir, entry_path) = temp_dir(files, entry);
    let out = Command::new(BIN)
        .args(["run", entry_path.to_str().unwrap()])
        .output()
        .unwrap();
    let result = (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn native_run_local_module_named_imports() {
    let (ok, stdout, stderr) = native_run(
        &[
            (
                "math.xulo",
                "pub fn add(a: number, b: number): number { a + b }\n\
                 pub const PI = 3.14\n",
            ),
            (
                "main.xulo",
                "mod math;\n\
                 use math::{add, PI};\n\
                 fn main() { print(add(2, 3)) print(PI) }\n",
            ),
        ],
        "main.xulo",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "5\n3.14\n");
}

#[test]
fn native_run_local_module_import_alias() {
    let (ok, stdout, stderr) = native_run(
        &[
            (
                "math.xulo",
                "pub fn add(a: number, b: number): number { a + b }\n",
            ),
            (
                "main.xulo",
                "mod math;\n\
                 use math::add as plus;\n\
                 fn main() { print(plus(1, 1)) }\n",
            ),
        ],
        "main.xulo",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "2\n");
}

#[test]
fn native_run_local_module_dependency_chain() {
    let (ok, stdout, stderr) = native_run(
        &[
            ("base.xulo", "pub fn base(): number { 10 }\n"),
            (
                "mid.xulo",
                "use base::base;\n\
                 pub fn mid(): number { base() + 5 }\n",
            ),
            (
                "main.xulo",
                "mod base;\n\
                 mod mid;\n\
                 use mid::mid;\n\
                 fn main() { print(mid()) }\n",
            ),
        ],
        "main.xulo",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "15\n");
}

#[test]
fn native_run_local_module_namespace_import() {
    let (ok, stdout, stderr) = native_run(
        &[
            (
                "cal.xulo",
                "pub const HOURS = 24\n\
                 pub fn hours(): number { HOURS }\n",
            ),
            (
                "main.xulo",
                "mod cal;\n\
                 use cal as c;\n\
                 fn main() { print(c::HOURS) print(c::hours()) }\n",
            ),
        ],
        "main.xulo",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "24\n24\n");
}

#[test]
fn native_run_local_module_named_import() {
    let (ok, stdout, stderr) = native_run(
        &[
            ("lib.xulo", "pub fn greet(): string { \"hi\" }\n"),
            (
                "main.xulo",
                "mod lib;\n\
                 use lib::greet;\n\
                 fn main() { print(greet()) }\n",
            ),
        ],
        "main.xulo",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "hi\n");
}

#[test]
fn native_run_imports_exported_enum_value() {
    // A runtime `use lib::Theme` of an exported enum must resolve in the
    // native runtime (`Theme::Dark` constructs by name), while the `.`-spelled
    // variant is ill-formed (spec `expressions/path-and-access.md`).
    for (name, entry_src) in [
        (
            "construct",
            "mod lib;\nuse lib::Theme;\nfn main() { print(Theme::Dark) }\n",
        ),
        (
            "member",
            "mod lib;\nuse lib::Theme;\nfn main() { print(Theme.Dark) }\n",
        ),
    ] {
        let (ok, stdout, stderr) = native_run(
            &[
                ("lib.xulo", "pub enum Theme { Dark Light }\n"),
                ("main.xulo", entry_src),
            ],
            "main.xulo",
        );
        if name == "member" {
            assert!(!ok, "{name}: stdout: {stdout}");
            assert!(
                stderr.contains("is an enum type with no value binding"),
                "{name}: stderr: {stderr}"
            );
        } else {
            assert!(ok, "{name}: stderr: {stderr}");
            assert_eq!(stdout, "Theme.Dark\n", "{name}");
        }
    }
}

#[test]
fn pub_bare_enum_name_imports_on_both_paths() {
    // `pub use { Color }` (the bare-names form) makes the enum importable,
    // like `pub enum Color` does. The `pub use` sits in the header; the
    // enum it re-exports is declared below.
    let (dir, entry) = temp_dir(
        &[
            (
                "lib.xulo",
                "pub use { Color };\npub enum Color { Red Blue }\n",
            ),
            (
                "main.xulo",
                "mod lib;\nuse lib::Color;\nfn main() { print(Color::Red) print(Color::Blue) }\n",
            ),
        ],
        "main.xulo",
    );
    let out = Command::new(BIN).arg("run").arg(&entry).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "Color.Red\nColor.Blue\n",
        ""
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn native_run_semantic_rejects_missing_export() {
    let (ok, stdout, stderr) = native_run(
        &[
            (
                "lib.xulo",
                "pub fn add(a: number, b: number): number { a + b }\n",
            ),
            (
                "main.xulo",
                "mod lib;\n\
                 use lib::nope;\n\
                 fn main() { print(nope(1)) }\n",
            ),
        ],
        "main.xulo",
    );
    assert!(!ok, "stdout: {stdout}");
    assert!(
        stderr.contains("does not export `nope`"),
        "stderr: {stderr}"
    );
}

#[test]
fn native_run_rejects_removed_default_keyword() {
    let (ok, _stdout, stderr) = native_run(
        &[("main.xulo", "default fn main() { print(1) }\n")],
        "main.xulo",
    );
    assert!(!ok);
    assert!(stderr.contains("syntax"), "stderr: {stderr}");
}

#[test]
fn native_run_external_import_binds_placeholder() {
    // An external package has no native values, but the run must not reject the
    // program: the manifest declares it (version-only, no local sources), its
    // imported names bind to `null` placeholders, and the entry runs.
    let (ok, stdout, stderr) = native_run(
        &[
            (
                "xulo.toml",
                "[dependencies]\n\"@xulo/ui\" = { version = \"1.2\", alias = \"ui\" }\n",
            ),
            (
                "main.xulo",
                "use ui::{useSignal};\n\
                 fn main() { print(1) }\n",
            ),
        ],
        "main.xulo",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "1\n");
}

#[test]
fn native_run_async_coroutines_interleave() {
    let (ok, stdout, stderr) = native_run(
        &[(
            "main.xulo",
            "async fn pause() { }\n\
                 async fn fetch(id: number): number {\n\
                     await pause()\n\
                     return id * 10\n\
                 }\n\
                 async fn main() {\n\
                     let a = fetch(1)\n\
                     let b = fetch(2)\n\
                     print(await a)\n\
                     print(await b)\n\
                 }\n",
        )],
        "main.xulo",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "10\n20\n");
}

#[test]
fn native_run_async_rejected_main_is_uncaught() {
    let (ok, _stdout, stderr) = native_run(
        &[("main.xulo", "async fn main() { panic(\"kaboom\") }\n")],
        "main.xulo",
    );
    assert!(!ok);
    assert!(
        stderr.contains("uncaught exception: kaboom"),
        "stderr: {stderr}"
    );
}

#[test]
fn native_run_ui_component_headless() {
    // The full UI example runs natively: `@State`, `@Effect`, component blocks,
    // `$` binding — the tree builds and effects fire, with no DOM to mount.
    let (ok, stdout, stderr) = native_run(
        &[
            (
                "xulo.toml",
                "[dependencies]\n\"@xulo/ui\" = { version = \"1.2\", alias = \"ui\" }\n",
            ),
            (
                "main.xulo",
                "use ui::{Screen, VStack, HStack, Text, Button, Input};\n\
                 fn Counter(): View {\n\
                     @State let count: number = 0\n\
                     @Effect fn() { print(\"mounted\") }\n\
                     VStack(spacing: 8) {\n\
                         Text(\"Count: \" + str(count))\n\
                         HStack(spacing: 4) {\n\
                             Button(onClick: fn() { count = count + 1 }) { Text(\"+\") }\n\
                             Button(onClick: fn() { count = count - 1 }) { Text(\"-\") }\n\
                         }\n\
                     }\n\
                 }\n\
                 fn NameField(): View {\n\
                     @State let name: string = \"\"\n\
                     VStack {\n\
                         Input(value: $name)\n\
                         Text(\"Hello, \" + name)\n\
                     }\n\
                 }\n\
                 fn main(): View {\n\
                     Screen {\n\
                         Counter()\n\
                         NameField()\n\
                     }\n\
                 }\n",
            ),
        ],
        "main.xulo",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "mounted\n");
}

#[test]
fn run_template_interpolation() {
    let file = temp_file(
        "interp.xulo",
        r#"
        fn main() {
            let name = "xulo"
            print(`Hello, ${name}!`)
            print(`2 + 3 = ${2 + 3}`)
            let b = true
            print(`bool: ${b}`)
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // `${list}` is E0212 like `str(list)` (spec: the two accept the same
    // operands); interpolation of base values stays as before.
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "Hello, xulo!\n2 + 3 = 5\nbool: true\n"
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_closed_range_loop_with_interpolation() {
    // The motivating example: a `for` loop over a closed range whose body
    // prints a JS-style template literal.
    let file = temp_file(
        "times.xulo",
        "fn main() { for i in 1...5 { print(`${i} times 5 is ${i * 5}`) } }",
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "1 times 5 is 5\n2 times 5 is 10\n3 times 5 is 15\n4 times 5 is 20\n5 times 5 is 25\n"
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_template_nested_and_escaped() {
    let file = temp_file(
        "interp_nested.xulo",
        "fn main() { let c = 1 print(`a${\n`b${c}`\n}${ {d: 1}.d }\\${not interp}`) }",
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "ab11${not interp}\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_template_multiline() {
    let file = temp_file("multiline.xulo", "fn main() { print(`line1\nline2`) }");
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "line1\nline2\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_template_stringify_object() {
    let file = temp_file(
        "interp_obj.xulo",
        r#"
        fn main() {
            let o = { a: 1, b: "two" }
            print(`object: ${o}`)
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "object: { a: 1, b: two }\n"
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_println() {
    let file = temp_file(
        "println.xulo",
        "fn main() { print(\"no marker\") println(\"one\") println(\"multi\", 1, true) println(`tpl ${2 + 3}`) }",
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "no marker\none\nmulti 1 true\ntpl 5\n"
    );
    let _ = std::fs::remove_file(&file);
}

// ---------------------------------------------------------------------------
// `xulo test` (spec testing.md)
// ---------------------------------------------------------------------------

#[test]
fn test_command_reports_pass_and_fail() {
    let file = temp_file(
        "tests_mixed.xulo",
        r#"
fn add(a: number, b: number): number { return a + b }

#Test
fn addition_works() {
    assert(add(2, 3) == 5, "2+3=5")
}

#Test
fn failing_test() {
    assert(add(1, 1) == 3, "one plus one is three")
}

fn main() {
    print("main must not run")
}
"#,
    );
    let out = Command::new(BIN).arg("test").arg(&file).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "stderr: {stderr}");
    assert!(
        stdout.contains("test addition_works ... ok"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("1 passed, 1 failed"), "stdout: {stdout}");
    assert!(
        stderr.contains("test failing_test ... FAILED") && stderr.contains("one plus one is three"),
        "stderr: {stderr}"
    );
    assert!(
        !stdout.contains("main must not run") && !stderr.contains("main must not run"),
        "`main` must not run under `xulo test`"
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn test_command_all_pass_exits_zero() {
    let file = temp_file(
        "tests_pass.xulo",
        r#"
#Test
fn first() {
    assert(true, "ok")
}

#Test
fn second() {
    assert(1 < 2, "ordering")
}
"#,
    );
    let out = Command::new(BIN).arg("test").arg(&file).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("test first ... ok"), "stdout: {stdout}");
    assert!(stdout.contains("test second ... ok"), "stdout: {stdout}");
    assert!(stdout.contains("2 passed, 0 failed"), "stdout: {stdout}");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn test_command_async_test_runs() {
    let file = temp_file(
        "tests_async.xulo",
        r#"
async fn double(x: number): number { return x * 2 }

#Test
async fn async_works() {
    let v = await double(21)
    assert(v == 42, "answer")
}
"#,
    );
    let out = Command::new(BIN).arg("test").arg(&file).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("test async_works ... ok"),
        "stdout: {stdout}"
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn test_command_inert_under_run() {
    let file = temp_file(
        "tests_inert.xulo",
        r#"
#Test
fn never_runs() {
    print("SHOULD NOT PRINT")
}

fn main() {
    print("main ran")
}
"#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "main ran\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn test_command_with_no_tests_exits_zero() {
    let file = temp_file("tests_none.xulo", "fn main() { print(\"hi\") }");
    let out = Command::new(BIN).arg("test").arg(&file).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("no tests found"), "stdout: {stdout}");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn native_run_pub_struct_cross_module() {
    let (ok, stdout, stderr) = native_run(
        &[
            (
                "lib.xulo",
                "pub struct Point { x: number, y: number }\n\
                 pub fn make(): Point { return Point(x: 1, y: 2) }\n",
            ),
            (
                "main.xulo",
                "mod lib;\n\
                 use lib::{Point, make};\n\
                 fn main() { let p = make() print(p.x) let q = Point(x: 5, y: 6) print(q.y) }\n",
            ),
        ],
        "main.xulo",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "1\n6\n");
}

#[test]
fn native_run_pub_use_reexports_across_modules() {
    let (ok, stdout, stderr) = native_run(
        &[
            ("lib.xulo", "pub fn greet(): string { \"hi\" }\n"),
            ("mid.xulo", "pub use lib::greet;\n"),
            (
                "main.xulo",
                "mod lib;\n\
                 mod mid;\n\
                 use mid::greet;\n\
                 fn main() { print(greet()) }\n",
            ),
        ],
        "main.xulo",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "hi\n");
}

#[test]
fn native_run_use_cycle_is_patched_before_main() {
    let (ok, stdout, stderr) = native_run(
        &[
            (
                "a.xulo",
                "use b::b;\n\
                 pub fn a(): number { 42 }\n\
                 pub fn call_b(): number { return b() }\n",
            ),
            (
                "b.xulo",
                "use a::a;\n\
                 pub fn b(): number { return a() }\n",
            ),
            (
                "main.xulo",
                "mod a;\n\
                 mod b;\n\
                 fn main() { print(b::b()) }\n",
            ),
        ],
        "main.xulo",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "42\n");
}

#[test]
fn native_run_pub_mod_namespace_reexport() {
    let (ok, stdout, stderr) = native_run(
        &[
            ("mid.xulo", "pub mod inner;\n"),
            ("mid/inner.xulo", "pub fn v(): number { 7 }\n"),
            (
                "main.xulo",
                "mod mid;\n\
                 use mid as m;\n\
                 fn main() { print(m::inner::v()) }\n",
            ),
        ],
        "main.xulo",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "7\n");
}

#[test]
fn native_run_called_import_is_not_reported_unused() {
    let (ok, stdout, stderr) = native_run(
        &[
            (
                "lib.xulo",
                "pub fn add(a: number, b: number): number { a + b }\n",
            ),
            (
                "main.xulo",
                "mod lib;\nuse lib::add;\nfn main() { print(add(2, 3)) }\n",
            ),
        ],
        "main.xulo",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "5\n");
    assert!(
        !stderr.contains("W0102"),
        "false-positive unused warning: {stderr}"
    );
}

#[test]
fn native_run_uncalled_import_still_warns_unused() {
    let (ok, _stdout, stderr) = native_run(
        &[
            (
                "lib.xulo",
                "pub fn add(a: number, b: number): number { a + b }\n",
            ),
            (
                "main.xulo",
                "mod lib;\nuse lib::add;\nfn main() { print(1) }\n",
            ),
        ],
        "main.xulo",
    );
    assert!(ok, "stderr: {stderr}");
    assert!(
        stderr.contains("W0102") && stderr.contains("`add`"),
        "expected W0102 for uncalled import: {stderr}"
    );
}
