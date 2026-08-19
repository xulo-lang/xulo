//! Integration tests for the multi-file `Workspace`: dependency-ordered
//! analysis, cross-module go-to-definition, import aliases, exports, and
//! cycle detection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use xulo_ide::analysis::Analysis;
use xulo_ide::object::OutlineKind;
use xulo_ide::workspace::{Located, Workspace};

const MAIN: &str = "/w/main.xulo";
const LIB: &str = "/w/lib.xulo";

fn sources() -> HashMap<PathBuf, String> {
    HashMap::from([
        (
            PathBuf::from(MAIN),
            "import { greet } from \"./lib\"\nfn main() {\n    let m = greet(\"hi\")\n}\n"
                .to_string(),
        ),
        (
            PathBuf::from(LIB),
            "pub fn greet(name: string): string {\n    return name\n}\n".to_string(),
        ),
    ])
}

fn pos(analysis: &Analysis, src: &str, needle: &str) -> xulo_ide::line_index::Pos {
    let offset = src.find(needle).expect("needle present");
    analysis.byte_to_position(offset).expect("in range")
}

#[test]
fn loads_and_analyzes_in_dependency_order() {
    let ws = Workspace::open(&sources(), Path::new(MAIN)).expect("open");
    assert_eq!(ws.modules().count(), 2);
    assert!(ws.module(Path::new(MAIN)).is_some());
    assert!(ws.module(Path::new(LIB)).is_some());
    let main = ws.analysis(Path::new(MAIN)).expect("main analysis");
    assert!(main.error.is_none(), "{:?}", main.error);
    let lib = ws.analysis(Path::new(LIB)).expect("lib analysis");
    assert!(lib.error.is_none(), "{:?}", lib.error);
    assert_eq!(ws.entry(), Path::new(MAIN));
}

#[test]
fn cross_module_go_to_definition_jumps_into_exporter() {
    let ws = Workspace::open(&sources(), Path::new(MAIN)).expect("open");
    let main = ws.analysis(Path::new(MAIN)).unwrap();
    let call_pos = pos(main, &main.source, "greet(\"hi\")");

    let located = ws
        .go_to_definition(Path::new(MAIN), call_pos)
        .expect("greet resolves across modules");
    assert_eq!(located.file, PathBuf::from(LIB));

    let lib = ws.analysis(Path::new(LIB)).unwrap();
    assert_eq!(located.range.start, pos(lib, &lib.source, "greet(name"));

    // The lib's declaration sits in its own module: same result from either
    // side, but anchored in lib.
    assert!(located.range.start.line < 2);
}

#[test]
fn local_definitions_stay_in_module() {
    let src = "\
fn helper(x: number): number {
    return x
}
fn main() {
    let y = helper(1)
}
";
    let mut all = sources();
    all.insert(PathBuf::from(MAIN), src.to_string());
    let ws = Workspace::open(&all, Path::new(MAIN)).expect("open");
    let main = ws.analysis(Path::new(MAIN)).unwrap();
    let use_pos = pos(main, src, "helper(1)");
    let located = ws
        .go_to_definition(Path::new(MAIN), use_pos)
        .expect("local resolves");
    assert_eq!(located.file, PathBuf::from(MAIN));
    assert_eq!(located.range.start, pos(main, src, "helper(x"));
}

#[test]
fn declaration_sites_resolve_to_themselves() {
    let src = "\
fn main() {
    let m = \"hi\"
}
";
    let mut all = sources();
    all.insert(PathBuf::from(MAIN), src.to_string());
    let ws = Workspace::open(&all, Path::new(MAIN)).expect("open");
    let main = ws.analysis(Path::new(MAIN)).unwrap();
    // Cursor on the declared name `m` resolves to `m` itself.
    let located = ws
        .go_to_definition(Path::new(MAIN), pos(main, src, "m ="))
        .expect("declaration resolves to itself");
    assert_eq!(located.file, PathBuf::from(MAIN));
    assert_eq!(located.range.start, pos(main, src, "m ="));
}

#[test]
fn references_include_the_declaration_site() {
    let src = "\
fn main() {
    let m = \"hi\"
    let n = m
}
";
    let mut all = sources();
    all.insert(PathBuf::from(MAIN), src.to_string());
    let ws = Workspace::open(&all, Path::new(MAIN)).expect("open");
    let main = ws.analysis(Path::new(MAIN)).unwrap();
    // Cursor exactly on the use of `m` on the right-hand side.
    let needle = "let n = m";
    let offset = main.source.find(needle).expect("needle present") + needle.len() - 1;
    let use_pos = main.byte_to_position(offset).expect("in range");
    let refs = main.find_references(use_pos);
    assert_eq!(refs.len(), 2, "declaration plus one use");
}

#[test]
fn import_aliases_resolve_to_exported_name() {
    let mut all = sources();
    all.insert(
        PathBuf::from(MAIN),
        "import { greet as g } from \"./lib\"\nfn main() {\n    let m = g(\"hi\")\n}\n".to_string(),
    );
    let ws = Workspace::open(&all, Path::new(MAIN)).expect("open");
    let main = ws.analysis(Path::new(MAIN)).unwrap();
    let located = ws
        .go_to_definition(Path::new(MAIN), pos(main, &main.source, "g(\"hi\")"))
        .expect("alias resolves");
    assert_eq!(located.file, PathBuf::from(LIB));
    let lib = ws.analysis(Path::new(LIB)).unwrap();
    assert_eq!(located.range.start, pos(lib, &lib.source, "greet(name"));
}

#[test]
fn exports_appear_in_outline_and_symbols() {
    let ws = Workspace::open(&sources(), Path::new(MAIN)).expect("open");
    let lib = ws.analysis(Path::new(LIB)).unwrap();
    let symbols = lib.document_symbols();
    let greet = symbols
        .iter()
        .find(|s| s.name == "greet" && s.kind == OutlineKind::Function)
        .expect("exported greet is outlined");
    assert_eq!(
        greet.selection_range.start,
        pos(lib, &lib.source, "greet(name")
    );
}

#[test]
fn namespace_import_anchors_at_target_start() {
    let mut all = sources();
    all.insert(
        PathBuf::from(MAIN),
        "import * as lib from \"./lib\"\nfn main() {\n    let m = lib.greet(\"hi\")\n}\n"
            .to_string(),
    );
    let ws = Workspace::open(&all, Path::new(MAIN)).expect("open");
    let main = ws.analysis(Path::new(MAIN)).unwrap();
    let located = ws
        .go_to_definition(Path::new(MAIN), pos(main, &main.source, "lib.greet"))
        .expect("namespace resolves");
    assert_eq!(located.file, PathBuf::from(LIB));
    assert_eq!(located.range.start, xulo_ide::line_index::Pos::default());
}

#[test]
fn cyclic_imports_are_rejected() {
    let mut all = HashMap::new();
    all.insert(
        PathBuf::from(MAIN),
        "import { b } from \"./b\"\npub fn a(): number { return 1 }\n".to_string(),
    );
    all.insert(
        PathBuf::from("/w/b.xulo"),
        "import { a } from \"./main\"\npub fn b(): number { return 2 }\n".to_string(),
    );
    let err = Workspace::open(&all, Path::new(MAIN)).expect_err("cycle is an error");
    assert!(err.message.contains("circular import"));
}

#[test]
fn missing_file_is_reported() {
    let all = HashMap::from([(PathBuf::from(MAIN), "fn main() {}\n".to_string())]);
    let err = Workspace::open(&all, Path::new("/w/nope.xulo")).expect_err("missing entry");
    assert!(err.message.contains("no such file"));
}

#[test]
fn failed_dependency_degrades_to_opaque() {
    // lib has a semantic error; main still analyzes (greet is opaque).
    let mut all = sources();
    all.insert(
        PathBuf::from(LIB),
        "pub fn greet(name: string): string {\n    return 42\n}\n".to_string(),
    );
    let ws = Workspace::open(&all, Path::new(MAIN)).expect("open");
    let lib = ws.analysis(Path::new(LIB)).unwrap();
    assert!(lib.error.is_some());
    let main = ws.analysis(Path::new(MAIN)).unwrap();
    assert!(main.error.is_none(), "{:?}", main.error);
}

#[test]
fn transitive_imports_work() {
    let mut all = sources();
    all.insert(
        PathBuf::from("/w/util.xulo"),
        "pub fn loud(s: string): string {\n    return s + \"!\"\n}\n".to_string(),
    );
    all.insert(
        PathBuf::from(LIB),
        "import { loud } from \"./util\"\npub fn greet(name: string): string {\n    return loud(name)\n}\n"
            .to_string(),
    );
    let ws = Workspace::open(&all, Path::new(MAIN)).expect("open");
    assert_eq!(ws.modules().count(), 3);
    let main = ws.analysis(Path::new(MAIN)).unwrap();
    assert!(main.error.is_none(), "{:?}", main.error);

    // loud used inside lib resolves to util, even via a two-hop chain.
    let lib = ws.analysis(Path::new(LIB)).unwrap();
    let located = ws
        .go_to_definition(Path::new(LIB), pos(lib, &lib.source, "loud(name"))
        .expect("transitive import resolves");
    assert_eq!(located.file, PathBuf::from("/w/util.xulo"));
    let util = ws.analysis(Path::new("/w/util.xulo")).unwrap();
    assert_eq!(
        located.range.start,
        pos(util, &util.source, "loud(s: string")
    );
}

#[test]
fn located_is_public_shape() {
    let ws = Workspace::open(&sources(), Path::new(MAIN)).expect("open");
    let main = ws.analysis(Path::new(MAIN)).unwrap();
    let located: Located = ws
        .go_to_definition(Path::new(MAIN), pos(main, &main.source, "greet(\"hi\")"))
        .unwrap();
    assert!(located.file.ends_with("lib.xulo"));
    assert!(located.range.start.character > 0);
}
