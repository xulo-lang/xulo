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
        std::fs::write(dir.join(name), content).unwrap();
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
fn build_writes_js_that_runs() {
    let src = temp_file("b.xulo", r#"fn main() { print("built") }"#);
    let out = temp_file("b.js", "");
    let result = Command::new(BIN)
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(result.status.success());

    let js = std::fs::read_to_string(&out).unwrap();
    assert!(js.contains("function main()"));
    assert!(js.contains("console.log(\"built\");"));
    assert!(js.contains("main();"));

    let node = Command::new("node").arg(&out).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&node.stdout), "built\n");

    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&out);
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
    // expression, which previously produced a spurious error); `run` forces
    // the buffered session to execute.
    let repl = Command::new(BIN)
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = repl.stdin.as_ref().unwrap();
    stdin
        .write_all(b"let x = 5\n\nx = x + 2\n\nprint(x)\n\nrun\nexit\n")
        .unwrap();
    let out = repl.wait_with_output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    // `print(x)` evaluates x to 7; re-running the session prints it again.
    assert!(
        text.matches('7').count() >= 2,
        "missing assignment result in:\n{text}"
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
        let count = 0
        count = count + 1

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
        "true\n{ tag: 'Success', value: 42 }\nXulo\n1\n2\ntrue\n"
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_file_with_external_import() {
    let dir = std::env::temp_dir().join(format!(
        "xulo_ext_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let pkg = dir.join("node_modules/@xulo/shapes");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(
        pkg.join("package.json"),
        r#"{ "name": "@xulo/shapes", "version": "0.0.1", "type": "module", "main": "index.js" }"#,
    )
    .unwrap();
    std::fs::write(
        pkg.join("index.js"),
        r#"export const box = (w, h) => ({ kind: "box", area: w * h });"#,
    )
    .unwrap();
    let src = dir.join("main.xulo");
    std::fs::write(
        &src,
        r#"
        import { box } from "@xulo/shapes"
        fn main() { let b = box(3, 4) print("area=" + str(b.area)) }
        "#,
    )
    .unwrap();
    let out = Command::new(BIN).arg("run").arg(&src).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "area=12\n");
    let _ = std::fs::remove_dir_all(&dir);
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
          let total = 0
          for i in 0..<5 { total = total + i }
          print(total)

          let c = 0
          while c < 3 { c = c + 1 }
          print(c)

          let name: string? = null
          print(name ?? "anon")

          let user: { name: string } = { name: "Xulo" }
          print(user.name)

          let obj = { a: 1 }
          let copy = { ...obj, b: 2 }
          print(copy.b)

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
fn run_multi_file_imports() {
    let (dir, entry) = temp_dir(
        &[
            (
                "math.xulo",
                r#"
                export fn add(a: number, b: number): number { return a + b }
                export const PI = 3.14
                "#,
            ),
            (
                "main.xulo",
                r#"
                import { add, PI } from "./math"
                import * as math from "./math"
                fn main() {
                    print(add(2, 3))
                    print(PI)
                    print(math.add(10, 1))
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
fn run_multi_file_default_and_async() {
    let (dir, entry) = temp_dir(
        &[
            (
                "greet.xulo",
                r#"
                export default fn greet(name: string): string { return "hi " + name }
                "#,
            ),
            (
                "load.xulo",
                r#"
                export fn load(n: number): async number {
                    let x = n * 2
                    return x
                }
                "#,
            ),
            (
                "main.xulo",
                r#"
                import greet from "./greet"
                import { load } from "./load"
                fn main(): async {
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
                "import { add, PI, Role } from \"./math\"\n\
                 fn main() { print(add(2, 3)) print(PI) print(Role::Admin) }\n",
            ),
        ],
        "main.xulo",
    );
    let out_dir = dir.join("bundle.js");
    let result = Command::new(BIN)
        .args([
            "build",
            entry.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let node = Command::new("node").arg(&out_dir).output().unwrap();
    let stdout = String::from_utf8_lossy(&node.stdout);
    assert_eq!(stdout.trim(), "5\n3.14\nRole.Admin");
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
    let file = temp_file("reserved.xulo", "fn main() { let struct = 1 }");
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("reserved keyword `struct`"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn allows_stdlib_type_names_as_identifiers() {
    // Standard-library type names are recommended against, but not reserved:
    // code that uses them as identifiers must keep working.
    let file = temp_file(
        "typenames.xulo",
        "fn main() {\n\
             let string = \"s\"\n\
             let number = 42\n\
             let list = [1]\n\
             print(string) print(number) print(list)\n\
         }",
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "s\n42\n[ 1 ]\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_module_import_destructure() {
    let (dir, entry) = temp_dir(
        &[
            (
                "util.xulo",
                "export fn double(x: number): number { return x * 2 }\n",
            ),
            (
                "main.xulo",
                "import { double } from \"./util\"\nfn main() { print(double(4)) }\n",
            ),
        ],
        "main.xulo",
    );
    let out_dir = dir.join("bundle.js");
    let result = Command::new(BIN)
        .args([
            "build",
            entry.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let js = std::fs::read_to_string(&out_dir).unwrap();
    assert!(js.contains("function double(x)"));
    assert!(js.contains("function main()"));
    assert!(js.contains("__mod0"));
    assert!(js.contains("main();"));

    let node = Command::new("node").arg(&out_dir).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&node.stdout), "8\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn external_type_only_import_is_erased_from_bundle() {
    let (dir, entry) = temp_dir(
        &[(
            "main.xulo",
            r#"
                import type { Config } from "lib-b"
                fn makeConfig(): Config { return "production" }
                "#,
        )],
        "main.xulo",
    );
    let out_dir = dir.join("bundle.js");
    let result = Command::new(BIN)
        .args([
            "build",
            entry.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let js = std::fs::read_to_string(&out_dir).unwrap();
    assert!(
        !js.contains("lib-b"),
        "type-only external import leaked into the bundle:\n{js}"
    );
    assert!(
        !js.contains("import "),
        "bundle should have no ESM imports:\n{js}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn external_runtime_import_is_kept_in_bundle() {
    let (dir, entry) = temp_dir(
        &[(
            "main.xulo",
            "import { helper } from \"lib-b\"\nfn main() { helper() }\n",
        )],
        "main.xulo",
    );
    let out_dir = dir.join("bundle.js");
    let result = Command::new(BIN)
        .args([
            "build",
            entry.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let js = std::fs::read_to_string(&out_dir).unwrap();
    assert!(js.contains("import { helper } from \"lib-b\";"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn external_import_is_deduplicated_across_modules() {
    // Two modules importing the same external package must emit a single
    // `import` statement with the merged specifiers (regression).
    let (dir, entry) = temp_dir(
        &[
            (
                "util.xulo",
                "import { double, triple } from \"lib-m\"\nexport fn f(x: number): number { return double(x) + triple(x) }\n",
            ),
            (
                "main.xulo",
                "import { square } from \"lib-m\"\nimport { f } from \"./util\"\nfn main() { print(f(square(2))) }\n",
            ),
        ],
        "main.xulo",
    );
    let out_dir = dir.join("bundle.js");
    let result = Command::new(BIN)
        .args([
            "build",
            entry.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let js = std::fs::read_to_string(&out_dir).unwrap();
    let import_lines = js.lines().filter(|l| l.starts_with("import ")).count();
    assert_eq!(import_lines, 1, "expected a single import statement:\n{js}");
    assert!(
        js.contains("from \"lib-m\"")
            && js.contains("double")
            && js.contains("triple")
            && js.contains("square"),
        "js:\n{js}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shared_runtime_emitted_once_across_modules() {
    // Several modules using reactive/range features still yield a single
    // `__runtime` declaration at the top of the bundle (regression).
    let (dir, entry) = temp_dir(
        &[
            (
                "a.xulo",
                "export fn comp(): Component { @State let n: number = 0 print(str(n)) }\n",
            ),
            (
                "util.xulo",
                "export fn sum(): number { let t = 0 for i in 0..<4 { t = t + i } return t }\n",
            ),
            (
                "main.xulo",
                "import { comp } from \"./a\"\nimport { sum } from \"./util\"\nfn main(): Component { comp() print(sum()) }\n",
            ),
        ],
        "main.xulo",
    );
    let out_dir = dir.join("bundle.js");
    let result = Command::new(BIN)
        .args([
            "build",
            entry.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let js = std::fs::read_to_string(&out_dir).unwrap();
    let runtime_count = js.matches("const __runtime =").count();
    assert_eq!(runtime_count, 1, "expected a single __runtime:\n{js}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_reports_missing_export() {
    let (dir, entry) = temp_dir(
        &[
            (
                "util.xulo",
                "export fn double(x: number): number { return x * 2 }\n",
            ),
            (
                "main.xulo",
                "import { triple } from \"./util\"\nfn main() { print(triple(1)) }\n",
            ),
        ],
        "main.xulo",
    );
    let out = Command::new(BIN).arg("check").arg(&entry).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no export named `triple`"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_reports_circular_import() {
    let (dir, entry) = temp_dir(
        &[
            (
                "a.xulo",
                "import { b } from \"./b\"\nexport fn a() { return 1 }\n",
            ),
            (
                "b.xulo",
                "import { a } from \"./a\"\nexport fn b() { return 2 }\n",
            ),
            (
                "main.xulo",
                "import { a } from \"./a\"\nfn main() { print(a()) }\n",
            ),
        ],
        "main.xulo",
    );
    let out = Command::new(BIN).arg("check").arg(&entry).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("circular"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_try_catch_and_throw() {
    let file = temp_file(
        "catch.xulo",
        r#"
        fn main() {
            try { throw "boom" } catch (e) { print("caught " + e) }
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "caught boom\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn type_only_enum_import_still_binds_the_value() {
    let (dir, entry) = temp_dir(
        &[
            ("colors.xulo", "export enum Kind { User Admin }\n"),
            (
                "main.xulo",
                r#"
                import type { Kind } from "./colors"
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
fn imported_function_supports_named_arguments() {
    let (dir, entry) = temp_dir(
        &[
            (
                "util.xulo",
                "export fn greet(name: string, times: number): string { name }\n",
            ),
            (
                "main.xulo",
                "import { greet } from \"./util\"\nfn main() { print(greet(times: 2, name: \"hi\")) }\n",
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
            let count = 0
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
        fn main(): async {
            let work = fn(): async { 21 * 2 }
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
            let sum = 0
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
        fn build(xs: list<number>, extra: number): list<number> {
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
        enum Action { Click, Submit(data: object), Cancel }
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
        fn f(a: number, b: string?, c: boolean = true): string {
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
        fn main(): Component {
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
fn build_component_with_external_ui() {
    let file = temp_file(
        "ui.xulo",
        r#"
        import { Screen, VStack, Text } from "@xulo/ui"

        fn main(): Component {
            @State let name: string = "Xulo"
            Screen {
                VStack(spacing: 16) {
                    Text("Name: " + name)
                }
            }
        }
        "#,
    );
    let out = temp_file("ui.mjs", "");
    let result = Command::new(BIN)
        .args(["build", file.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let js = std::fs::read_to_string(&out).unwrap();
    assert!(js.contains("import { Screen, VStack, Text } from \"@xulo/ui\";"));
    assert!(js.contains("__component"));
    assert!(js.contains("children: ["));
    assert!(js.contains("name.get()"));
    assert!(js.contains("__xulo_mount"));
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn run_component_with_forwarded_children() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xulo_ui_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let pkg = dir.join("node_modules/@xulo/ui");
    std::fs::create_dir_all(&pkg).unwrap();
    let shim = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/node_modules/@xulo/ui");
    std::fs::copy(shim.join("package.json"), pkg.join("package.json")).unwrap();
    std::fs::copy(shim.join("index.js"), pkg.join("index.js")).unwrap();

    let entry = dir.join("app.xulo");
    std::fs::write(
        &entry,
        r#"
        import { Screen, VStack, Text } from "@xulo/ui"

        fn Card(title: string, children: list<Component>): Component {
            VStack {
                Text(title, weight: "bold")
                children
            }
        }

        fn main(): Component {
            @State let name: string = "Xulo"
            Screen {
                Card(title: "Profile") {
                    Text("Hello, " + name)
                }
            }
        }
        "#,
    )
    .unwrap();

    let out = dir.join("app.mjs");
    let result = Command::new(BIN)
        .args([
            "build",
            entry.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let node = Command::new("node").arg(&out).output().unwrap();
    assert!(
        node.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&node.stderr)
    );
    let stdout = String::from_utf8_lossy(&node.stdout).to_string();
    // The forwarded slot renders both the custom component's own text title
    // and the caller-supplied child (`children` is flattened).
    assert!(stdout.contains("weight: bold"), "stdout: {stdout}");
    assert!(stdout.contains("Hello, Xulo"), "stdout: {stdout}");
    assert!(stdout.contains("<Screen>"), "stdout: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_rejects_decorator_outside_component() {
    let file = temp_file("bad.xulo", "fn main() { @State let count: number = 0 }");
    let out = Command::new(BIN).arg("check").arg(&file).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("returning `Component`"));
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_component_loop_var_shadowing() {
    let file = temp_file(
        "shadow.xulo",
        r#"
        fn main(): Component {
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
        fn main(): Component {
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
            let xs: list<number> = [1, 2, 3]
            xs[0] = 10
            xs[2] = 30
            let user: { age: number } = { age: 20 }
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
            print(str(null))
        }
        "#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "v=3.5\ntrue\nnull\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_nested_loops_and_ranges() {
    let file = temp_file(
        "loops.xulo",
        r#"
        fn main() {
            let total = 0
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
            let n = 0
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
            let people: list<{ name: string, age: number }> = [{ name: "a", age: 1 }, { name: "b", age: 2 }]
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
            let u: { a: { b: number } }? = null
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
    assert_eq!(String::from_utf8_lossy(&out.stdout), "undefined\ntrue\n");
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
        "{ tag: 'A', value: [ 1, 'x' ] }\na1x\n"
    );
    let _ = std::fs::remove_file(&file);
}
