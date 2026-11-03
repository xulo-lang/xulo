//! The `std::fs` expansion: the remaining eighteen members of the
//! whole-file surface — append, exclusive create, touch, times, mode,
//! realpath, stat, copy, rename, directory listing/walking/removal, the byte
//! twins, and the temporary creators. Every test owns a scratch directory
//! under the system temp dir and removes it at the end.

use std::fs;
use std::path::{Path, PathBuf};

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

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "xulo-runtime-fs4b-{}-{}",
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
fn append_grows_and_creates() {
    let dir = scratch("append");
    let file = dir.join("log.txt");
    let out = run_ok(&format!(
        r#"
        fn main() {{
            let path = "{}"
            match std::fs::append(path, "first ") {{ Result::Ok(_) => {{}} Result::Err(e) => print(e.message) }}
            match std::fs::append(path, "second") {{ Result::Ok(_) => {{}} Result::Err(e) => print(e.message) }}
            match std::fs::read(path) {{ Result::Ok(t) => print(t) Result::Err(e) => print(e.message) }}
            match std::fs::append(path, "") {{ Result::Ok(_) => print("empty ok") Result::Err(e) => print(e.message) }}
        }}
        "#,
        p(&file)
    ));
    assert_eq!(out, vec!["first second", "empty ok"]);
    assert_eq!(fs::read_to_string(&file).unwrap(), "first second");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn create_new_is_exclusive() {
    let dir = scratch("creatnew");
    let file = dir.join("once.txt");
    let out = run_ok(&format!(
        r#"
        fn main() {{
            let path = "{}"
            match std::fs::create_new(path, "first") {{ Result::Ok(_) => print("ok1") Result::Err(e) => print(e.message) }}
            match std::fs::create_new(path, "second") {{ Result::Ok(_) => print("bad") Result::Err(_) => print("err2") }}
            match std::fs::read(path) {{ Result::Ok(t) => print(t) Result::Err(e) => print(e.message) }}
        }}
        "#,
        p(&file)
    ));
    assert_eq!(out, vec!["ok1", "err2", "first"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn touch_and_set_times() {
    let dir = scratch("touch");
    let file = dir.join("stamp.txt");
    let out = run_ok(&format!(
        r#"
        fn main() {{
            let path = "{}"
            match std::fs::touch(path) {{ Result::Ok(_) => {{}} Result::Err(e) => print(e.message) }}
            match std::fs::read(path) {{ Result::Ok(t) => print(t == "") Result::Err(e) => print(e.message) }}
            let w = std::fs::write(path, "keep")
            match std::fs::touch(path) {{ Result::Ok(_) => {{}} Result::Err(e) => print(e.message) }}
            match std::fs::read(path) {{ Result::Ok(t) => print(t) Result::Err(e) => print(e.message) }}
            match std::fs::set_times(path, 0) {{ Result::Ok(_) => {{}} Result::Err(e) => print(e.message) }}
            match std::fs::stat(path) {{ Result::Ok(i) => print(i.modified) Result::Err(e) => print(e.message) }}
        }}
        "#,
        p(&file)
    ));
    assert_eq!(out, vec!["true", "keep", "0"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn set_mode_domain_and_bits() {
    let dir = scratch("mode");
    let file = dir.join("tool");
    let out = run_ok(&format!(
        r#"
        fn main() {{
            let path = "{}"
            let w = std::fs::write(path, "x")
            match std::fs::set_mode(path, 4096) {{ Result::Ok(_) => print("bad") Result::Err(_) => print("range") }}
            match std::fs::set_mode(path, 0o755) {{ Result::Ok(_) => print("set") Result::Err(e) => print(e.message) }}
            match std::fs::stat(path) {{ Result::Ok(i) => print(i.mode == 493) Result::Err(e) => print(e.message) }}
        }}
        "#,
        p(&file)
    ));
    assert_eq!(out, vec!["range", "set", "true"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn stat_reports_kind_size_and_stamp() {
    let dir = scratch("stat");
    let file = dir.join("a.txt");
    let out = run_ok(&format!(
        r#"
        fn main() {{
            let path = "{}"
            let w = std::fs::write(path, "hello")
            match std::fs::stat(path) {{ Result::Ok(i) => {{ print(i.kind); print(i.size) }} Result::Err(e) => print(e.message) }}
            match std::fs::stat("{}") {{ Result::Ok(i) => print(i.kind) Result::Err(e) => print(e.message) }}
            match std::fs::stat(path + "-missing") {{ Result::Ok(_) => print("bad") Result::Err(_) => print("missing") }}
        }}
        "#,
        p(&file),
        p(&dir)
    ));
    assert_eq!(out, vec!["file", "5", "directory", "missing"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn realpath_is_absolute_and_missing_fails() {
    let dir = scratch("realpath");
    let file = dir.join("a.txt");
    let out = run_ok(&format!(
        r#"
        fn main() {{
            let w = std::fs::write("{file}", "x")
            match std::fs::realpath("{file}") {{ Result::Ok(r) => print(r) Result::Err(e) => print(e.message) }}
            match std::fs::realpath("{file}-missing") {{ Result::Ok(_) => print("bad") Result::Err(_) => print("missing") }}
        }}
        "#,
        file = p(&file)
    ));
    assert!(out[0].starts_with("/"), "{}", out[0]);
    assert_eq!(out[1], "missing");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn copy_and_rename_replace_and_refuse_self() {
    let dir = scratch("copy");
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    let c = dir.join("c.txt");
    let out = run_ok(&format!(
        r#"
        fn main() {{
            let w = std::fs::write("{a}", "payload")
            match std::fs::copy("{a}", "{b}") {{ Result::Ok(_) => print("copied") Result::Err(e) => print(e.message) }}
            match std::fs::read("{b}") {{ Result::Ok(t) => print(t) Result::Err(e) => print(e.message) }}
            match std::fs::copy("{a}", "{a}") {{ Result::Ok(_) => print("bad") Result::Err(_) => print("self-copy") }}
            match std::fs::rename("{b}", "{c}") {{ Result::Ok(_) => print("renamed") Result::Err(e) => print(e.message) }}
            print(std::fs::exists("{b}"))
            match std::fs::rename("{c}", "{c}") {{ Result::Ok(_) => print("bad") Result::Err(_) => print("self-rename") }}
        }}
        "#,
        a = p(&a),
        b = p(&b),
        c = p(&c)
    ));
    assert_eq!(
        out,
        vec![
            "copied",
            "payload",
            "self-copy",
            "renamed",
            "false",
            "self-rename"
        ]
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn read_dir_lists_bare_names_sorted() {
    let dir = scratch("readdir");
    fs::write(dir.join("b.txt"), "b").unwrap();
    fs::write(dir.join("a.txt"), "a").unwrap();
    let out = run_ok(&format!(
        r#"
        fn main() {{
            match std::fs::read_dir("{}") {{ Result::Ok(names) => print(names) Result::Err(e) => print(e.message) }}
            match std::fs::read_dir("{}-missing") {{ Result::Ok(_) => print("bad") Result::Err(_) => print("missing") }}
        }}
        "#,
        p(&dir),
        p(&dir)
    ));
    assert_eq!(out, vec!["[a.txt, b.txt]", "missing"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn walk_lists_start_then_descendants() {
    let dir = scratch("walk");
    fs::create_dir(dir.join("sub")).unwrap();
    fs::write(dir.join("top.txt"), "t").unwrap();
    fs::write(dir.join("sub/nested.txt"), "n").unwrap();
    let out = run_ok(&format!(
        r#"
        fn main() {{
            match std::fs::walk("{}") {{ Result::Ok(paths) => {{ print(paths[0] == "{}"); print(paths.len) }} Result::Err(e) => print(e.message) }}
        }}
        "#,
        p(&dir),
        p(&dir)
    ));
    assert_eq!(out, vec!["true", "4"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn remove_dir_takes_the_tree() {
    let dir = scratch("rmdir");
    let tree = dir.join("tree");
    fs::create_dir_all(tree.join("sub")).unwrap();
    fs::write(tree.join("sub/f.txt"), "x").unwrap();
    let out = run_ok(&format!(
        r#"
        fn main() {{
            match std::fs::remove_dir("{tree}") {{ Result::Ok(_) => print("gone") Result::Err(e) => print(e.message) }}
            print(std::fs::exists("{tree}"))
            match std::fs::remove_dir("{tree}") {{ Result::Ok(_) => print("bad") Result::Err(_) => print("missing") }}
            let w = std::fs::write("{dir}/f.txt", "x")
            match std::fs::remove_dir("{dir}/f.txt") {{ Result::Ok(_) => print("bad") Result::Err(_) => print("file") }}
        }}
        "#,
        tree = p(&tree),
        dir = p(&dir)
    ));
    assert_eq!(out, vec!["gone", "false", "missing", "file"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn byte_round_trip_survives_non_utf8() {
    let dir = scratch("bytes");
    let blob = dir.join("blob.bin");
    let out = run_ok(&format!(
        r#"
        fn main() {{
            let path = "{}"
            match std::fs::write_bytes(path, [0x00, 0xFF, 0x7F, 0x45]) {{ Result::Ok(_) => {{}} Result::Err(e) => print(e.message) }}
            match std::fs::read_bytes(path) {{ Result::Ok(bs) => print(bs) Result::Err(e) => print(e.message) }}
            match std::fs::append_bytes(path, [1, 2]) {{ Result::Ok(_) => {{}} Result::Err(e) => print(e.message) }}
            match std::fs::read_bytes(path) {{ Result::Ok(bs) => print(bs.len) Result::Err(e) => print(e.message) }}
            match std::fs::read(path) {{ Result::Ok(_) => print("bad") Result::Err(_) => print("not utf8") }}
        }}
        "#,
        p(&blob)
    ));
    assert_eq!(out, vec!["[0, 255, 127, 69]", "6", "not utf8"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn temp_creators_yield_unique_empty_entries() {
    let dir = scratch("temp");
    let out = run_ok(&format!(
        r#"
        fn main() {{
            match std::fs::temp_file("xulo4b_") {{
                Result::Ok(tp) => {{ print(tp.split("xulo4b_").len == 2); match std::fs::read(tp) {{ Result::Ok(t) => print(t == "") Result::Err(e) => print(e.message) }} let rm = std::fs::remove(tp) }}
                Result::Err(e) => print(e.message)
            }}
            match std::fs::temp_dir() {{
                Result::Ok(td) => {{ match std::fs::read_dir(td) {{ Result::Ok(n) => print(n.len == 0) Result::Err(e) => print(e.message) }} let rm = std::fs::remove_dir(td) }}
                Result::Err(e) => print(e.message)
            }}
            match std::fs::temp_file_in("{dir}", "in_") {{
                Result::Ok(tp) => {{ print(tp.split("in_").len == 2); let rm = std::fs::remove(tp) }}
                Result::Err(e) => print(e.message)
            }}
            match std::fs::temp_dir_in("{dir}-missing", "x") {{ Result::Ok(_) => print("bad") Result::Err(_) => print("missing dir") }}
        }}
        "#,
        dir = p(&dir)
    ));
    assert_eq!(out, vec!["true", "true", "true", "true", "missing dir"]);
    let _ = fs::remove_dir_all(&dir);
}
