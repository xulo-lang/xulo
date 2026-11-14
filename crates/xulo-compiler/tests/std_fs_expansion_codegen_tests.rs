//! The `std::fs` expansion on the compiled path: append, exclusive create,
//! touch, times/mode, realpath, stat, copy, rename, listing, walking,
//! remove_dir, and the temporary creators. Byte members stay off the compiled
//! path (list literals have no compiled representation yet).

use std::fs;
use std::path::{Path, PathBuf};

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

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "xulo-compiler-fs4b-{}-{}",
        std::process::id(),
        label
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn p(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn append_and_create_new_run_on_the_compiled_path() {
    let dir = scratch("append");
    let file = dir.join("log.txt");
    let out = compile_and_run(&format!(
        r#"
fn main() {{
    let path = "{file}"
    let a = std::fs::append(path, "one")
    let b = std::fs::append(path, " two")
    match std::fs::read(path) {{ Result::Ok(t) => print(t) Result::Err(e) => print(e.message) }}
    match std::fs::create_new(path, "x") {{ Result::Ok(_) => print("bad") Result::Err(_) => print("exclusive") }}
}}
"#,
        file = p(&file)
    ));
    assert_eq!(out, vec!["one two", "exclusive"]);
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn stat_touch_times_mode_on_the_compiled_path() {
    let dir = scratch("stat");
    let file = dir.join("a.txt");
    let out = compile_and_run(&format!(
        r#"
fn main() {{
    let path = "{file}"
    let w = std::fs::write(path, "hello")
    let t = std::fs::touch(path)
    let st = std::fs::set_times(path, 0)
    match std::fs::stat(path) {{
        Result::Ok(i) => {{ print(i.kind); print(i.size); print(i.modified) }}
        Result::Err(e) => print(e.message)
    }}
    let m = std::fs::set_mode(path, 0o644)
}}
"#,
        file = p(&file)
    ));
    assert_eq!(out, vec!["file", "5", "0"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn copy_rename_realpath_remove_dir_on_the_compiled_path() {
    let dir = scratch("move");
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    let sub = dir.join("sub");
    fs::write(&a, "payload").unwrap();
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("n.txt"), "n").unwrap();
    let out = compile_and_run(&format!(
        r#"
fn main() {{
    match std::fs::realpath("{a}") {{ Result::Ok(r) => print(r.len > 0) Result::Err(e) => print(e.message) }}
    let c = std::fs::copy("{a}", "{b}")
    let rn = std::fs::rename("{b}", "{dir}/moved.txt")
    match std::fs::read("{dir}/moved.txt") {{ Result::Ok(t) => print(t) Result::Err(e) => print(e.message) }}
    let rd = std::fs::remove_dir("{sub}")
    print(std::fs::exists("{sub}"))
}}
"#,
        a = p(&a),
        b = p(&b),
        dir = p(&dir),
        sub = p(&sub)
    ));
    assert_eq!(out, vec!["true", "payload", "false"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn read_dir_and_walk_on_the_compiled_path() {
    let dir = scratch("list");
    fs::create_dir(dir.join("sub")).unwrap();
    fs::write(dir.join("a.txt"), "a").unwrap();
    fs::write(dir.join("sub/n.txt"), "n").unwrap();
    let out = compile_and_run(&format!(
        r#"
fn main() {{
    match std::fs::read_dir("{dir}") {{ Result::Ok(names) => {{ print(names.len); print(names[0]) }} Result::Err(e) => print(e.message) }}
    match std::fs::walk("{dir}") {{ Result::Ok(paths) => print(paths.len) Result::Err(e) => print(e.message) }}
}}
"#,
        dir = p(&dir)
    ));
    assert_eq!(out, vec!["2", "a.txt", "4"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn temp_file_optional_prefix_on_the_compiled_path() {
    let dir = scratch("temp");
    let out = compile_and_run(&format!(
        r#"
fn main() {{
    match std::fs::temp_file_in("{dir}", "aot_") {{
        Result::Ok(tp) => {{ print(tp.split("aot_").len == 2); let rm = std::fs::remove(tp) }}
        Result::Err(e) => print(e.message)
    }}
    match std::fs::temp_dir_in("{dir}", "d_") {{
        Result::Ok(td) => {{ let rm = std::fs::remove_dir(td) }}
        Result::Err(e) => print(e.message)
    }}
}}
"#,
        dir = p(&dir)
    ));
    assert_eq!(out, vec!["true"]);
    let _ = fs::remove_dir_all(&dir);
}
