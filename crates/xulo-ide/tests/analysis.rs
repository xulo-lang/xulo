//! Integration tests for the analyzer crate: LineIndex conversions,
//! go-to-definition / find-references, hover, the document outline, and
//! diagnostics mapping.

use xulo_ide::analysis::Analysis;
use xulo_ide::diagnostics::Severity;
use xulo_ide::line_index::{LineIndex, Pos};
use xulo_ide::object::OutlineKind;
use xulo_ide::{analyze_source, queries};

/// Byte offset of the `n`-th (0-based) occurrence of `needle` in `src`.
fn nth(src: &str, needle: &str, n: usize) -> usize {
    src.match_indices(needle).nth(n).expect("needle present").0
}

/// The LSP position of the `n`-th occurrence of `needle`.
fn pos(analysis: &Analysis, src: &str, needle: &str, n: usize) -> Pos {
    analysis
        .byte_to_position(nth(src, needle, n))
        .expect("in range")
}

const SAMPLE: &str = "\
fn greet(name: string): string {
    return name
}
let input = \"hi\"
let out = greet(input)
";

#[test]
fn line_index_round_trips_utf16() {
    let src = "这是一行\n第二行😀";
    let index = LineIndex::new(src);
    assert_eq!(
        index.byte_to_position(src, 0),
        Some(Pos {
            line: 0,
            character: 0
        })
    );
    assert_eq!(
        index.byte_to_position(src, 12),
        Some(Pos {
            line: 0,
            character: 4
        })
    );
    assert_eq!(
        index.byte_to_position(src, 13),
        Some(Pos {
            line: 1,
            character: 0
        })
    );
    assert_eq!(
        index.byte_to_position(src, 22),
        Some(Pos {
            line: 1,
            character: 3
        })
    );
    assert_eq!(
        index.byte_to_position(src, 26),
        Some(Pos {
            line: 1,
            character: 5
        })
    );

    assert_eq!(
        index.position_to_byte(
            src,
            Pos {
                line: 0,
                character: 4
            }
        ),
        Some(12)
    );
    assert_eq!(
        index.position_to_byte(
            src,
            Pos {
                line: 1,
                character: 0
            }
        ),
        Some(13)
    );
    assert_eq!(
        index.position_to_byte(
            src,
            Pos {
                line: 1,
                character: 3
            }
        ),
        Some(22)
    );
    assert_eq!(
        index.position_to_byte(
            src,
            Pos {
                line: 1,
                character: 5
            }
        ),
        Some(26)
    );
    assert_eq!(
        index.position_to_byte(
            src,
            Pos {
                line: 9,
                character: 0
            }
        ),
        None
    );
}

#[test]
fn analyze_source_succeeds() {
    let analysis = analyze_source(SAMPLE);
    assert!(
        analysis.error.is_none(),
        "unexpected error: {:?}",
        analysis.error
    );
    assert!(analysis.program().is_some());
    assert!(analysis.result().is_some());
    assert!(analysis.diagnostics().is_empty());
}

#[test]
fn go_to_definition_resolves_uses() {
    let analysis = analyze_source(SAMPLE);
    let src = SAMPLE;

    // `greet(input)` → `fn greet` (name spans bytes 3..8 on line 0).
    let def = analysis
        .go_to_definition(pos(&analysis, src, "greet(", 0))
        .expect("greet resolves");
    assert_eq!(
        def.range.start,
        Pos {
            line: 0,
            character: 3
        }
    );
    assert_eq!(
        def.range.end,
        Pos {
            line: 0,
            character: 8
        }
    );

    // `input` in the call → the `let input` declaration on line 3.
    let decl_start = pos(&analysis, src, "input =", 0);
    let def = analysis
        .go_to_definition(pos(&analysis, src, "input)", 0))
        .expect("input resolves");
    assert_eq!(def.range.start, decl_start);
    assert_eq!(
        def.range.end,
        Pos {
            line: decl_start.line,
            character: decl_start.character + 5
        }
    );

    // Clicking directly on a declaration reports itself as its definition.
    let self_def = analysis
        .go_to_definition(pos(&analysis, src, "input =", 0))
        .expect("declaration resolves to itself");
    assert_eq!(self_def.range, def.range);
}

#[test]
fn find_references_includes_declaration_and_uses() {
    let analysis = analyze_source(SAMPLE);
    let src = SAMPLE;

    // From the call site, `input` has its declaration + this use.
    let refs = analysis.find_references(pos(&analysis, src, "input)", 0));
    assert_eq!(refs.len(), 2);
    let decl = pos(&analysis, src, "input =", 0);
    assert!(refs.iter().any(|l| l.range.start == decl), "missing decl");
    let use_ = pos(&analysis, src, "input)", 0);
    assert!(refs.iter().any(|l| l.range.start == use_), "missing use");

    // From the declaration site the same two locations come back.
    let refs2 = analysis.find_references(decl);
    assert_eq!(refs.len(), refs2.len());
}

#[test]
fn shadowing_resolves_to_innermost_declaration() {
    let src = "\
let x = 1
fn f(): number {
    let x = 2
    return x
}
let y = x
";
    let analysis = analyze_source(src);
    assert!(analysis.error.is_none(), "{:?}", analysis.error);

    let inner_byte = nth(src, "return x", 0) + "return ".len();
    let inner_x = analysis.byte_to_position(inner_byte).unwrap();
    let outer = analysis
        .byte_to_position(nth(src, "y = x", 0) + "y = ".len())
        .unwrap();
    let inner_decl = pos(&analysis, src, "x = 2", 0);
    let outer_decl = pos(&analysis, src, "x = 1", 0);

    assert_eq!(
        analysis.go_to_definition(inner_x).unwrap().range.start,
        inner_decl
    );
    assert_eq!(
        analysis.go_to_definition(outer).unwrap().range.start,
        outer_decl
    );

    let inner_refs = analysis.find_references(inner_x);
    assert_eq!(inner_refs.len(), 2);
    assert!(
        inner_refs
            .iter()
            .all(|l| l.range.start == inner_decl || l.range.start == inner_x)
    );
    let outer_refs = analysis.find_references(outer);
    assert_eq!(outer_refs.len(), 2);
    assert!(
        outer_refs
            .iter()
            .all(|l| l.range.start == outer_decl || l.range.start == outer)
    );
}

#[test]
fn closure_captures_outer_binding() {
    let src = "\
fn apply(): number {
    let base = 10
    let f = fn(x: number): number {
        return x + base
    }
    return 0
}
";
    let analysis = analyze_source(src);
    assert!(analysis.error.is_none(), "{:?}", analysis.error);

    let base_decl = pos(&analysis, src, "base = 10", 0);
    let base_use = analysis
        .byte_to_position(nth(src, "+ base", 0) + 2)
        .unwrap();
    let hover = analysis.hover_at(base_use).expect("hover");
    assert_eq!(hover.name, "base");
    assert_eq!(hover.type_name.as_deref(), Some("number"));
    assert_eq!(hover.def.unwrap().start, base_decl);
    assert_eq!(
        analysis.go_to_definition(base_use).unwrap().range.start,
        base_decl
    );
}

#[test]
fn hover_reports_names_and_literal_types() {
    let src = "\
let num = 42
let doubled = num * 2
";
    let analysis = analyze_source(src);
    assert!(analysis.error.is_none(), "{:?}", analysis.error);

    let on_use = analysis
        .hover_at(pos(&analysis, src, "num *", 0))
        .expect("hover on use");
    assert_eq!(on_use.name, "num");
    assert_eq!(on_use.kind, "variable");
    assert_eq!(on_use.type_name.as_deref(), Some("number"));
    assert_eq!(on_use.def.unwrap().start, pos(&analysis, src, "num =", 0));

    // A literal has no name-use; the inferred expression type is shown.
    let on_literal = analysis
        .hover_at(pos(&analysis, src, "42", 0))
        .expect("hover on literal");
    assert_eq!(on_literal.kind, "expression");
    assert_eq!(on_literal.type_name.as_deref(), Some("number"));
}

#[test]
fn document_symbols_builds_outline() {
    let src = "\
type Rectangle = object
enum Color {
    Red,
    Green
}
trait Area {
    fn area(self): number
}
impl Area for Rectangle {
    fn area(self): number {
        return 0
    }
}
fn main() {
    let a = 1
}
@Environment let dark: boolean
";
    let analysis = analyze_source(src);
    // Outline works off the AST even when semantic analysis reports errors.
    assert!(analysis.program().is_some());

    let symbols = analysis.document_symbols();
    let kinds: Vec<(&str, OutlineKind, usize)> = symbols
        .iter()
        .map(|s| (s.name.as_str(), s.kind, s.children.len()))
        .collect();
    assert!(kinds.contains(&("Rectangle", OutlineKind::TypeAlias, 0)));
    assert!(kinds.contains(&("Color", OutlineKind::Enum, 2)));
    assert!(kinds.contains(&("Area", OutlineKind::Trait, 1)));
    assert!(
        symbols
            .iter()
            .any(|s| s.kind == OutlineKind::Impl && s.children.len() == 1)
    );
    assert!(kinds.contains(&("main", OutlineKind::Function, 0)));
    assert!(kinds.contains(&("dark", OutlineKind::Variable, 0)));

    let color = symbols.iter().find(|s| s.name == "Color").unwrap();
    assert_eq!(color.children[0].name, "Red");
    assert_eq!(color.children[0].kind, OutlineKind::EnumMember);
    let area = symbols.iter().find(|s| s.name == "Area").unwrap();
    assert_eq!(area.children[0].name, "area");
    assert_eq!(area.children[0].kind, OutlineKind::Method);
}

#[test]
fn definitions_returns_both_views() {
    let analysis = analyze_source(SAMPLE);
    let result = analysis.definitions(pos(&analysis, SAMPLE, "input)", 0));
    assert_eq!(result.definitions.len(), 1);
    assert_eq!(result.references.len(), 2);
    assert_eq!(result.definitions[0].range, result.references[0].range);
}

#[test]
fn symbols_group_by_declaration() {
    let analysis = analyze_source(SAMPLE);
    let symbols = analysis.symbols();
    let input = symbols
        .iter()
        .find(|s| s.name == "input")
        .expect("input symbol");
    assert_eq!(input.uses.len(), 2); // declaration + the `greet(input)` use
    let greet = symbols
        .iter()
        .find(|s| s.name == "greet")
        .expect("greet symbol");
    assert_eq!(greet.uses.len(), 1);
    assert!(greet.def.is_some());
}

#[test]
fn diagnostics_map_errors_and_warnings() {
    let semantic = analyze_source("fn main() {\n    let x = 1\n    let y = x + not_a_thing\n}\n");
    let diags = semantic.diagnostics();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].severity, Severity::Error);
    assert_eq!(diags[0].code.as_deref(), Some("E0003"));
    assert_eq!(
        diags[0].range.start,
        pos(
            &semantic,
            "fn main() {\n    let x = 1\n    let y = x + not_a_thing\n}\n",
            "not_a_thing",
            0
        )
    );

    let parse = analyze_source("let x =");
    let diags = parse.diagnostics();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code.as_deref(), Some("E0002"));
    assert_eq!(diags[0].severity, Severity::Error);

    let lex = analyze_source("let # = 1");
    let diags = lex.diagnostics();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code.as_deref(), Some("E0001"));

    // A trailing `expr;` in a typed function warns instead of erroring.
    let warning_src = "\
fn f(): number {
    let x = 1
    x;
}
";
    let warning = analyze_source(warning_src);
    assert!(warning.error.is_none(), "{:?}", warning.error);
    let diags = warning.diagnostics();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].severity, Severity::Warning);
    assert_eq!(diags[0].code.as_deref(), Some("W0001"));
    assert!(diags[0].message.contains("ignored return value"));
    assert_eq!(diags[0].range.start, pos(&warning, warning_src, "x;", 0));
}

#[test]
fn queries_are_noops_without_successful_analysis() {
    let analysis = analyze_source("let x =");
    assert!(analysis.program().is_none());
    assert!(analysis.document_symbols().is_empty());
    assert!(analysis.symbols().is_empty());
    assert!(analysis.find_references(Pos::default()).is_empty());
    assert!(analysis.go_to_definition(Pos::default()).is_none());
    assert!(analysis.hover_at(Pos::default()).is_none());
    assert_eq!(analysis.diagnostics().len(), 1);
}

#[test]
fn position_conversions_through_analysis() {
    let analysis = analyze_source(SAMPLE);
    let offset = nth(SAMPLE, "greet(", 0);
    let p = analysis.byte_to_position(offset).unwrap();
    assert_eq!(analysis.position_to_byte(p), Some(offset));
    assert_eq!(
        p,
        Pos {
            line: 0,
            character: 3
        }
    );
}

#[test]
fn location_type_is_public() {
    let analysis = analyze_source(SAMPLE);
    let loc = analysis
        .go_to_definition(pos(&analysis, SAMPLE, "greet(", 0))
        .unwrap();
    let loc2: queries::Location = loc;
    assert_eq!(
        loc2.range.start,
        Pos {
            line: 0,
            character: 3
        }
    );
}

#[test]
fn component_hover_and_definition() {
    let src = "\
fn Card(title: string): View {
    View { Text(title) }
}
fn main() {
    View {
        Card(\"hi\")
        Text(\"x\")
    }
}
";
    let analysis = analyze_source(src);
    assert!(analysis.error.is_none(), "{:?}", analysis.error);

    // User component: colors like `View`, definition still resolves.
    let card = analysis
        .hover_at(pos(&analysis, src, "Card(\"hi\")", 0))
        .expect("hover on user component");
    assert_eq!(card.name, "Card");
    assert_eq!(card.kind, "component");
    assert_eq!(card.type_name.as_deref(), Some("View"));
    let def_start = analysis
        .go_to_definition(pos(&analysis, src, "Card(\"hi\")", 0))
        .unwrap()
        .range
        .start;
    assert_eq!(def_start, pos(&analysis, src, "Card", 0));

    // Builtin component: hover works, no definition to jump to.
    let text = analysis
        .hover_at(pos(&analysis, src, "Text(\"x\")", 0))
        .expect("hover on builtin component");
    assert_eq!(text.name, "Text");
    assert_eq!(text.kind, "component");
    assert!(text.def.is_none());
    assert!(
        analysis
            .go_to_definition(pos(&analysis, src, "Text(\"x\")", 0))
            .is_none()
    );
}

#[test]
fn hover_and_definition_survive_errors() {
    let src = "\
fn main() {
    let good = 1
    let bad = not_a_function(2)
    let after = good + 1
}
";
    let analysis = analyze_source(src);
    assert!(analysis.error.is_some(), "file carries a semantic error");

    // Hover still works on a healthy variable even though the file has an error.
    let hover = analysis
        .hover_at(pos(&analysis, src, "after =", 0))
        .expect("hover on healthy variable");
    assert_eq!(hover.kind, "variable");
    assert_eq!(hover.type_name.as_deref(), Some("number"));

    // Definition resolution still works.
    let def = analysis
        .go_to_definition(pos(&analysis, src, "good +", 0))
        .expect("definition on healthy variable");
    assert_eq!(def.range.start, pos(&analysis, src, "good =", 0));
}

#[test]
fn hover_shows_signature_params_and_doc_comment() {
    let src = "\
type User = object
// A card that renders a greeting.
// Title is uppercased.
fn Card(title: string, user: User): View {
    View { Text(title) }
}
fn main() {
    View {
        Card(\"hi\", { name: \"a\" })
    }
}
";
    let analysis = analyze_source(src);
    assert!(analysis.error.is_none(), "{:?}", analysis.error);

    // Component usage: preview the factory's signature, params and comment.
    let usage = analysis
        .hover_at(pos(&analysis, src, "Card(\"hi\",", 0))
        .expect("hover on component usage");
    assert_eq!(usage.kind, "component");
    assert_eq!(usage.type_name.as_deref(), Some("View"));
    assert_eq!(
        usage.signature.as_deref(),
        Some("fn Card(title: string, user: User): View")
    );
    assert_eq!(
        usage.params,
        vec![
            ("title".to_string(), "string".to_string()),
            ("user".to_string(), "User".to_string()),
        ]
    );
    assert_eq!(
        usage.comment.as_deref(),
        Some("A card that renders a greeting.\nTitle is uppercased.")
    );

    // Function declaration hover also previews the signature.
    let decl = analysis
        .hover_at(pos(&analysis, src, "Card(", 0))
        .expect("hover on function declaration");
    assert_eq!(decl.kind, "function");
    assert_eq!(
        decl.signature.as_deref(),
        Some("fn Card(title: string, user: User): View")
    );
}
