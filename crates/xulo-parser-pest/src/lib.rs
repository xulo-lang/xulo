//! pest.rs-based parser for Xulo (Proof of Concept)
//!
//! This crate demonstrates using pest for parsing Xulo source code.
//! It parses directly from text and shows the parse tree structure.

use pest::iterators::Pair;
use pest::Parser;

// Include the generated parser
#[allow(clippy::all)]
mod grammar {
    use pest_derive::Parser;

    #[derive(Parser)]
    #[grammar = "xulo.pest"]
    pub struct XuloParser;
}

pub use grammar::Rule;
pub use grammar::XuloParser;

/// Parse Xulo source code and return the raw pest pairs.
pub fn parse(source: &str) -> Result<Vec<Pair<'_, Rule>>, String> {
    let pairs = XuloParser::parse(Rule::program, source)
        .map_err(|e| format!("parse error: {}", e))?;

    Ok(pairs.collect())
}

/// Pretty-print the parse tree.
pub fn print_tree(pairs: &[Pair<'_, Rule>], indent: usize) {
    for pair in pairs {
        let rule = pair.as_rule();
        let span = pair.as_span();
        let text = pair.as_str();

        println!(
            "{}{:?} [{}..{}] = {:?}",
            "  ".repeat(indent),
            rule,
            span.start(),
            span.end(),
            if text.len() > 50 {
                format!("{}...", &text[..50])
            } else {
                text.to_string()
            }
        );

        let inner: Vec<Pair<'_, Rule>> = pair.clone().into_inner().collect();
        if !inner.is_empty() {
            print_tree(&inner, indent + 1);
        }
    }
}
