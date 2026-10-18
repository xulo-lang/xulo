//! Module-system regression tests: claims (`mod`), `use` resolution, header
//! and manifest rules, initialization order, and use cycles — the loader
//! diagnostics (E0607/E0610) and the phase-2 checks (E0607/E0611) that guard
//! the package tree (spec `modules/*.md`).

use std::path::{Path, PathBuf};

/// A fresh package directory named after the test.
fn temp_pkg(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "xulo_ms_{}_{name}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, content: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

fn load_pkg(
    dir: &Path,
) -> Result<xulo_compiler::module::LoadedModules, xulo_core::error::XuloError> {
    xulo_compiler::module::load(&dir.join("main.xulo"))
}

fn load_err(dir: &Path) -> xulo_core::error::XuloError {
    match load_pkg(dir) {
        Ok(_) => panic!("expected the package to fail to load"),
        Err(err) => err,
    }
}

fn analyze_pkg(
    dir: &Path,
) -> Result<Vec<xulo_core::error::XuloError>, xulo_core::error::XuloError> {
    let mut loaded = load_pkg(dir)?;
    xulo_compiler::module::analyze(&mut loaded, false)
}

#[test]
fn missing_claimed_file_is_e0607() {
    let dir = temp_pkg("missing_claim");
    write(&dir, "main.xulo", "mod lib;\nfn main() {}\n");
    let err = load_err(&dir);
    assert_eq!(err.code(), "E0607", "message: {}", err.message);
    assert!(
        err.message.contains("no file at"),
        "message: {}",
        err.message
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn self_claim_is_e0610() {
    let dir = temp_pkg("self_claim");
    write(&dir, "main.xulo", "mod main;\nfn main() {}\n");
    let err = load_err(&dir);
    assert_eq!(err.code(), "E0610", "message: {}", err.message);
    assert!(
        err.message
            .contains("would claim the file that declares it"),
        "message: {}",
        err.message
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn duplicate_mod_name_in_one_file_is_e0104() {
    let dir = temp_pkg("dup_mod");
    write(&dir, "main.xulo", "mod a;\nmod a;\nfn main() {}\n");
    write(&dir, "a.xulo", "pub fn f() {}\n");
    let err = load_err(&dir);
    assert_eq!(err.code(), "E0104", "message: {}", err.message);
    assert!(
        err.message
            .contains("`mod a` is declared twice in this module"),
        "message: {}",
        err.message
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn same_file_claimed_by_two_packages_is_e0610() {
    // Two manifest dependencies rooted at the same directory: the second
    // claim of the same file is `E0610` (spec source-files.md, one claiming
    // module per file).
    let dir = temp_pkg("dup_claim");
    write(
        &dir,
        "xulo.toml",
        "[dependencies]\n\
         dep1 = { version = \"0.1\", path = \"./dep\" }\n\
         dep2 = { version = \"0.1\", path = \"./dep\" }\n",
    );
    write(&dir, "dep/main.xulo", "pub fn f() {}\n");
    write(&dir, "main.xulo", "fn main() {}\n");
    let err = load_err(&dir);
    assert_eq!(err.code(), "E0610", "message: {}", err.message);
    assert!(
        err.message.contains("already the body of"),
        "message: {}",
        err.message
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unresolvable_head_is_e0607() {
    let dir = temp_pkg("bad_head");
    write(&dir, "main.xulo", "use nope::x;\nfn main() {}\n");
    let err = analyze_pkg(&dir).unwrap_err();
    assert_eq!(err.code(), "E0607", "message: {}", err.message);
    assert!(err.message.contains("nope"), "message: {}", err.message);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn super_at_package_root_is_e0607() {
    let dir = temp_pkg("super_root");
    write(&dir, "main.xulo", "use super::x;\nfn main() {}\n");
    let err = analyze_pkg(&dir).unwrap_err();
    assert_eq!(err.code(), "E0607", "message: {}", err.message);
    assert!(
        err.message
            .contains("cannot resolve `super` at the package root"),
        "message: {}",
        err.message
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn use_cycles_are_accepted() {
    // spec `modules/import-export.md`: `use` cycles are legal (two-phase
    // fill); analysis must accept mutual imports between siblings.
    let dir = temp_pkg("cycle");
    write(
        &dir,
        "main.xulo",
        "mod a;\nmod b;\nfn main() { print(a::aa()) }\n",
    );
    write(&dir, "a.xulo", "use b::bb;\npub fn aa(): Number { bb() }\n");
    write(&dir, "b.xulo", "use a::aa;\npub fn bb(): Number { aa() }\n");
    let warnings = analyze_pkg(&dir).expect("mutual use must analyze");
    assert!(
        !warnings.iter().any(|w| w.code() == "W0102"),
        "cycle members are used: {:?}",
        warnings.iter().map(|w| &w.message).collect::<Vec<_>>()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn top_level_forward_read_is_e0611() {
    // `a` initializes before `b` (declaration order), so `a`'s top-level
    // read of `b::k` observes an uninitialized module (spec
    // `modules/source-files.md`, initialization order).
    let dir = temp_pkg("fwd_read");
    write(&dir, "main.xulo", "mod a;\nmod b;\nfn main() {}\n");
    write(&dir, "a.xulo", "use b::k;\nlet x = k\n");
    write(&dir, "b.xulo", "pub const k = 1\n");
    let err = analyze_pkg(&dir).unwrap_err();
    assert_eq!(err.code(), "E0611", "message: {}", err.message);
    assert!(
        err.message.contains("is used before"),
        "message: {}",
        err.message
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_order_is_post_order() {
    // Children initialize before parents: a's child first, then a, then the
    // entry (post-order walk of the claim tree).
    let dir = temp_pkg("init_order");
    write(&dir, "main.xulo", "mod a;\nfn main() {}\n");
    write(&dir, "a.xulo", "mod a1;\npub fn f() {}\n");
    write(&dir, "a/a1.xulo", "pub fn g() {}\n");
    let loaded = load_pkg(&dir).unwrap();
    let ids: Vec<&str> = loaded.modules.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["a::a1", "a", ""], "post-order init: {ids:?}");
    assert_eq!(loaded.entry, 2, "the entry module initializes last");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn manifest_alias_resolves_package_head() {
    let dir = temp_pkg("manifest_alias");
    write(
        &dir,
        "xulo.toml",
        "[dependencies]\n\"@xulo/ui\" = { version = \"1.2\", alias = \"ui\" }\n",
    );
    write(&dir, "main.xulo", "use ui::{widget};\nfn main() {}\n");
    let mut loaded = load_pkg(&dir).unwrap();
    assert_eq!(
        loaded.world.packages.get("ui").map(String::as_str),
        Some("@ui"),
        "the alias is the dependency's only name in source"
    );
    xulo_compiler::module::analyze(&mut loaded, false)
        .expect("a version-only dependency resolves opaquely");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn manifest_quoted_key_without_alias_is_error() {
    let dir = temp_pkg("manifest_noalias");
    write(
        &dir,
        "xulo.toml",
        "[dependencies]\n\"@xulo/ui\" = \"1.2\"\n",
    );
    write(&dir, "main.xulo", "fn main() {}\n");
    let err = load_err(&dir);
    assert!(
        err.message.contains("needs an `alias`"),
        "message: {}",
        err.message
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn manifest_duplicate_path_name_is_error() {
    let dir = temp_pkg("manifest_dup");
    write(
        &dir,
        "xulo.toml",
        "[dependencies]\n\
         clap = \"4.0\"\n\
         \"@x/ui\" = { version = \"1.0\", alias = \"clap\" }\n",
    );
    write(&dir, "main.xulo", "fn main() {}\n");
    let err = load_err(&dir);
    assert!(
        err.message.contains("share the path name `clap`"),
        "message: {}",
        err.message
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn path_dependency_initializes_first() {
    let dir = temp_pkg("path_dep");
    write(
        &dir,
        "xulo.toml",
        "[dependencies]\ndep = { version = \"0.1\", path = \"./dep\" }\n",
    );
    write(&dir, "dep/main.xulo", "pub fn helper(): Number { 7 }\n");
    write(
        &dir,
        "main.xulo",
        "use dep::helper;\nfn main() { print(helper()) }\n",
    );
    let loaded = load_pkg(&dir).unwrap();
    assert_eq!(loaded.modules[0].id, "@dep", "declared dependencies first");
    assert_eq!(
        loaded.modules[loaded.entry].id, "",
        "the entry package root runs after its dependencies"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn path_dependency_without_main_is_error() {
    let dir = temp_pkg("path_dep_nomain");
    write(
        &dir,
        "xulo.toml",
        "[dependencies]\ndep = { version = \"0.1\", path = \"./dep\" }\n",
    );
    write(&dir, "dep/lib.xulo", "pub fn helper(): Number { 7 }\n");
    write(&dir, "main.xulo", "fn main() {}\n");
    let err = load_err(&dir);
    assert!(
        err.message.contains("no main.xulo under"),
        "message: {}",
        err.message
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn manifest_path_name_shadowing_root_module_is_e0104() {
    // A spelling that is both a dependency's path name and a root module of
    // this package is `E0104` (spec import-export.md, "Name collisions").
    let dir = temp_pkg("name_collide");
    write(
        &dir,
        "xulo.toml",
        "[dependencies]\n\"@xulo/ui\" = { version = \"1.2\", alias = \"ui\" }\n",
    );
    write(&dir, "ui.xulo", "pub fn f() {}\n");
    write(&dir, "main.xulo", "mod ui;\nfn main() {}\n");
    let err = analyze_pkg(&dir).unwrap_err();
    assert_eq!(err.code(), "E0104", "message: {}", err.message);
    assert!(
        err.message.contains("manifest path name"),
        "message: {}",
        err.message
    );
    let _ = std::fs::remove_dir_all(&dir);
}
