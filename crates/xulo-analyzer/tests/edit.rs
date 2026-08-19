//! Unit tests for the LSP server's incremental change application
//! (`lib::edit::apply_changes`), which mutes UTF-16 editor coordinates back
//! into byte offsets via the analyzer's line index.

use xulo_analyzer::edit::{TextEdit, apply_changes};
use xulo_ide::line_index::Pos;

fn pos(line: u32, character: u32) -> Pos {
    Pos { line, character }
}

#[test]
fn full_replacement_wins_without_range() {
    let mut doc = "fn main() {}".to_string();
    apply_changes(
        &mut doc,
        &[TextEdit {
            range: None,
            text: "fn main() {}".to_string(),
        }],
    );
    assert_eq!(doc, "fn main() {}");
}

#[test]
fn utf16_columns_map_to_byte_offsets() {
    // Line 1 holds an emoji (2 UTF-16 units) before the ASCII word to edit.
    let mut doc = "let greeting = \"hi\"\nconst emoji = 😀 + x".to_string();
    // Remove "x" at line 1: before it sit 15 BMP chars plus an emoji that
    // occupies UTF-16 units 14..16, so "x" starts at UTF-16 unit 19.
    apply_changes(
        &mut doc,
        &[TextEdit {
            range: Some((pos(1, 19), pos(1, 20))),
            text: "y".to_string(),
        }],
    );
    assert!(doc.ends_with("+ y"));
    assert!(!doc.contains("+ x"));
}

#[test]
fn splice_replaces_between_us_ascii_positions() {
    let mut doc = "fn main() {\n    let a = 1\n}".to_string();
    // Replace `a` (line 1, char 8) with `answer`.
    apply_changes(
        &mut doc,
        &[TextEdit {
            range: Some((pos(1, 8), pos(1, 9))),
            text: "answer".to_string(),
        }],
    );
    assert_eq!(doc, "fn main() {\n    let answer = 1\n}");
}

#[test]
fn consecutive_changes_apply_in_order() {
    let mut doc = "ab".to_string();
    apply_changes(
        &mut doc,
        &[
            TextEdit {
                range: Some((pos(0, 0), pos(0, 1))),
                text: "x".to_string(),
            },
            TextEdit {
                range: Some((pos(0, 1), pos(0, 2))),
                text: "y".to_string(),
            },
        ],
    );
    assert_eq!(doc, "xy");
}

#[test]
fn out_of_bounds_edit_clamps_to_document_end() {
    let mut doc = "abc".to_string();
    apply_changes(
        &mut doc,
        &[TextEdit {
            range: Some((pos(5, 0), pos(5, 3))),
            text: "X".to_string(),
        }],
    );
    assert_eq!(doc, "abcX");
}
