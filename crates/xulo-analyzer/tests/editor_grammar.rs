//! The VS Code assets shipped next to this server (`editors/vscode/`) are
//! hand-maintained copies of facts the compiler owns. These tests pin them to
//! the sources of truth so a language change cannot silently drift them apart
//! again — the TextMate grammar once listed retired keywords (`try`, `catch`,
//! `throw`, `import`, `from`), used `@Name` for `#Name` attributes, and had no
//! template-literal rule at all.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use xulo_analyzer::semantic::{TOKEN_MODIFIERS, TOKEN_TYPES};

fn editors(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../editors/vscode")
        .join(rel)
}

fn parser(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../xulo-parser")
        .join(rel)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Every `"spelling"` inside a pest rule body of the form `name = _{ ... }`.
fn pest_quoted_words(source: &str, rule: &str) -> BTreeSet<String> {
    let start = source
        .find(&format!("\n{rule} = "))
        .unwrap_or_else(|| panic!("pest rule `{rule}` not found"));
    let body_start = source[start..].find('{').expect("rule body opens") + start;
    let body_end = source[body_start..].find('}').expect("rule body closes") + body_start;
    let body = &source[body_start + 1..body_end];
    let mut words = BTreeSet::new();
    let mut rest = body;
    while let Some(open) = rest.find('"') {
        let Some(close) = rest[open + 1..].find('"') else {
            break;
        };
        words.insert(rest[open + 1..open + 1 + close].to_string());
        rest = &rest[open + 1 + close + 1..];
    }
    words
}

/// The literal words a `match` alternation names, e.g. `\b(?:and|or)\b` and
/// `(a|b|c)` both yield `a`, `b`, `c`. Non-alternation regexes yield nothing.
fn alternation_words(matcher: &str) -> BTreeSet<String> {
    let open_needle = "\\b(?:";
    let inner = match matcher.find(open_needle) {
        Some(at) => {
            let after = &matcher[at + open_needle.len()..];
            match after.find(")\\b") {
                Some(end) => &after[..end],
                None => after,
            }
        }
        None => matcher,
    };
    inner
        .split('|')
        .map(str::trim)
        .filter(|word| {
            !word.is_empty()
                && word
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .map(str::to_string)
        .collect()
}

/// Words the grammar paints as a keyword-ish scope, gathered from the whole
/// repository of the TextMate grammar.
fn grammar_keyword_words(grammar: &serde_json::Value) -> BTreeSet<String> {
    fn walk(node: &serde_json::Value, words: &mut BTreeSet<String>) {
        match node {
            serde_json::Value::Object(map) => {
                let name = map.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let interesting = name.starts_with("keyword.")
                    || name.starts_with("storage.type.primitive")
                    || name.starts_with("constant.language")
                    || name.starts_with("invalid.illegal.reserved");
                if interesting && let Some(matcher) = map.get("match").and_then(|v| v.as_str()) {
                    words.extend(alternation_words(matcher));
                }
                for value in map.values() {
                    walk(value, words);
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|item| walk(item, words)),
            _ => {}
        }
    }
    let mut words = BTreeSet::new();
    walk(grammar, &mut words);
    words
}

fn grammar_text() -> serde_json::Value {
    let raw = read(&editors("syntaxes/xulo.tmLanguage.json"));
    serde_json::from_str(&raw).expect("xulo.tmLanguage.json is valid JSON")
}

#[test]
fn grammar_highlights_every_pest_keyword() {
    let pest = read(&parser("grammar.pest"));
    let keywords = pest_quoted_words(&pest, "kw_word");
    let grammar_words = grammar_keyword_words(&grammar_text());
    let missing: Vec<&String> = keywords.difference(&grammar_words).collect();
    assert!(
        missing.is_empty(),
        "TextMate grammar does not highlight these keywords: {missing:?}"
    );
}

#[test]
fn grammar_marks_every_reserved_word_as_illegal() {
    let pest = read(&parser("grammar.pest"));
    let reserved = pest_quoted_words(&pest, "reserved_word");
    let grammar = grammar_text();
    let mut illegal = BTreeSet::new();
    collect_scope_words(&grammar, "invalid.illegal.reserved", &mut illegal);
    let missing: Vec<&String> = reserved.difference(&illegal).collect();
    assert!(
        missing.is_empty(),
        "reserved words not marked `invalid.illegal.reserved`: {missing:?}"
    );
    // Reserved words and keywords must not overlap: a keyword is never illegal.
    let keywords = pest_quoted_words(&pest, "kw_word");
    let overlap: Vec<&String> = keywords.intersection(&illegal).collect();
    assert!(overlap.is_empty(), "keywords marked illegal: {overlap:?}");
}

fn collect_scope_words(node: &serde_json::Value, scope_prefix: &str, words: &mut BTreeSet<String>) {
    match node {
        serde_json::Value::Object(map) => {
            if map
                .get("name")
                .and_then(|v| v.as_str())
                .is_some_and(|name| name.starts_with(scope_prefix))
                && let Some(matcher) = map.get("match").and_then(|v| v.as_str())
            {
                words.extend(alternation_words(matcher));
            }
            for value in map.values() {
                collect_scope_words(value, scope_prefix, words);
            }
        }
        serde_json::Value::Array(items) => items
            .iter()
            .for_each(|item| collect_scope_words(item, scope_prefix, words)),
        _ => {}
    }
}

#[test]
fn grammar_drops_keywords_that_are_no_longer_keywords() {
    // Retired in favor of other spellings: `import`/`from` with the string
    // importer gone, `try`/`catch`/`throw` never in the language.
    let retired = ["try", "catch", "throw", "import", "from"];
    let words = grammar_keyword_words(&grammar_text());
    let still_present: Vec<&&str> = retired.iter().filter(|w| words.contains(**w)).collect();
    assert!(
        still_present.is_empty(),
        "grammar still colours retired spellings as keywords: {still_present:?}"
    );
}

#[test]
fn grammar_covers_the_syntax_the_lexer_defines() {
    let raw = read(&editors("syntaxes/xulo.tmLanguage.json"));
    // Backtick templates with `${...}` interpolation.
    assert!(
        raw.contains("string.template.xulo"),
        "no template-literal rule"
    );
    assert!(
        raw.contains("punctuation.definition.template-expression.begin.xulo"),
        "template interpolation is not highlighted"
    );
    // Declaration attributes are `#Name`, not `@Name`.
    assert!(
        raw.contains("punctuation.definition.attribute.xulo"),
        "no `#Name` attribute rule"
    );
    assert!(
        raw.contains("storage.modifier.component.xulo"),
        "no `@State` component-annotation rule"
    );
    // Block comments and both quote styles exist in the lexer.
    assert!(raw.contains("comment.block.xulo"), "no block-comment rule");
    assert!(raw.contains("string.quoted.single.xulo"), "no '…' rule");
    // Numeric forms: decimal / hex / binary / octal / float.
    for form in [
        "constant.numeric.hex",
        "constant.numeric.binary",
        "constant.numeric.octal",
    ] {
        assert!(raw.contains(form), "no {form} rule");
    }
    // The intrinsic function names of spec `builtins/intrinsic-functions.md`.
    let mut intrinsics = BTreeSet::new();
    collect_scope_words(
        &grammar_text(),
        "support.function.intrinsic",
        &mut intrinsics,
    );
    let expected: BTreeSet<String> = ["assert", "print", "println", "str"]
        .into_iter()
        .map(str::to_string)
        .collect();
    assert_eq!(intrinsics, expected, "intrinsics highlighted");
}

#[test]
fn package_json_references_files_that_ship() {
    let package: serde_json::Value =
        serde_json::from_str(&read(&editors("package.json"))).expect("package.json is valid JSON");
    assert_eq!(
        package["main"].as_str(),
        Some("./extension.js"),
        "extension entry point"
    );
    let grammar = package["contributes"]["grammars"][0]["path"]
        .as_str()
        .expect("grammar path");
    let language = package["contributes"]["languages"][0]["configuration"]
        .as_str()
        .expect("language configuration path");
    for rel in [
        grammar.trim_start_matches("./"),
        language.trim_start_matches("./"),
    ] {
        assert!(
            editors(rel).is_file(),
            "package.json references missing `{rel}`"
        );
    }
    assert_eq!(
        package["contributes"]["languages"][0]["id"].as_str(),
        Some("xulo")
    );
}

#[test]
fn language_configuration_is_valid_and_covers_the_lexical_delimiters() {
    let raw = read(&editors("language-configuration.json"));
    let config: serde_json::Value = serde_json::from_str(&raw).expect("language config is JSON");
    assert_eq!(config["comments"]["lineComment"], "//");
    assert_eq!(config["comments"]["blockComment"][0], "/*");
    assert_eq!(config["comments"]["blockComment"][1], "*/");
    let pairs: Vec<(String, String)> = config["autoClosingPairs"]
        .as_array()
        .expect("autoClosingPairs")
        .iter()
        .map(|p| {
            (
                p["open"].as_str().unwrap().into(),
                p["close"].as_str().unwrap().into(),
            )
        })
        .collect();
    for (open, close) in [
        ("{", "}"),
        ("[", "]"),
        ("(", ")"),
        ("\"", "\""),
        ("'", "'"),
        ("`", "`"),
    ] {
        assert!(
            pairs.iter().any(|(o, c)| o == open && c == close),
            "`{open}` is not auto-closed with `{close}`"
        );
    }
    assert!(config["indentationRules"]["increaseIndentPattern"].is_string());
    assert!(config["indentationRules"]["decreaseIndentPattern"].is_string());
}

#[test]
fn client_semantic_legend_matches_the_server_legend() {
    // `extension.js` hardcodes the legend it advertises to the server; the
    // server advertises its own. A mismatch silently mis-colours every token.
    let source = read(&editors("extension.js"));
    let client_types = js_string_list(&source, "tokenTypes");
    let client_modifiers = js_string_list(&source, "tokenModifiers");
    assert_eq!(client_types, TOKEN_TYPES, "semantic token types drifted");
    assert_eq!(
        client_modifiers, TOKEN_MODIFIERS,
        "semantic token modifiers drifted"
    );
}

/// The single-quoted string elements of a `key: [ ... ]` list in `extension.js`.
fn js_string_list(source: &str, key: &str) -> Vec<String> {
    let at = source
        .find(&format!("{key}: ["))
        .unwrap_or_else(|| panic!("`{key}: [` not found in extension.js"));
    let body = &source[at + key.len() + 2..];
    let end = body
        .find(']')
        .unwrap_or_else(|| panic!("`{key}` list not closed"));
    let mut words = Vec::new();
    let mut rest = &body[..end];
    while let Some(open) = rest.find('\'') {
        let Some(close) = rest[open + 1..].find('\'') else {
            break;
        };
        words.push(rest[open + 1..open + 1 + close].to_string());
        rest = &rest[open + 1 + close + 1..];
    }
    words
}
