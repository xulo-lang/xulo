//! The `regex` module: compiled patterns and their matches, the engine
//! behind the language's `RegExp` values (website spec
//! `regular-expressions.mdx`). This layer is pure Rust — no interpreter
//! values — so the runtime can build `RegExp`/`RegexMatch` objects from the
//! data here. Patterns are plain strings; there is no `/pattern/` literal.

use regex::{Regex, RegexBuilder};

/// A compiled pattern plus its flags. `g` is remembered rather than baked
/// into the engine (the regex crate has no such flag): it decides whether
/// `replace` substitutes every match or only the first.
#[derive(Debug)]
pub struct RegExp {
    re: Regex,
    has_g: bool,
}

/// One match: the matched text, its byte span in the input (for splicing a
/// closure replacement), and capture groups `1..n` as owned strings. A group
/// that did not participate is `None`, which the language reads as `null`.
pub struct Match {
    pub value: String,
    pub start: usize,
    pub end: usize,
    pub groups: Vec<Option<String>>,
}

impl Match {
    /// Capture group `index` (1-based, as `group(index)` is written);
    /// `None` when the index is out of range or the group did not match.
    pub fn group(&self, index: usize) -> Option<&str> {
        index
            .checked_sub(1)
            .and_then(|i| self.groups.get(i))
            .and_then(|g| g.as_deref())
    }
}

fn match_of(caps: &regex::Captures<'_>) -> Match {
    let whole = caps.get(0).expect("captures_iter yields group 0");
    Match {
        value: whole.as_str().to_string(),
        start: whole.start(),
        end: whole.end(),
        groups: (1..caps.len())
            .map(|i| caps.get(i).map(|m| m.as_str().to_string()))
            .collect(),
    }
}

/// `RegExp::compile(pattern, flags)`: flags are `i` (case-insensitive), `g`
/// (replace all matches), `m` (multiline `^`/`$`), `u` (unicode — the engine
/// default, accepted for the spec's flag table). An unknown flag or an
/// illegal pattern is an error message; compilation never panics.
pub fn compile(pattern: &str, flags: &str) -> Result<RegExp, String> {
    let mut case_insensitive = false;
    let mut multi_line = false;
    let mut has_g = false;
    for flag in flags.chars() {
        match flag {
            'i' => case_insensitive = true,
            'g' => has_g = true,
            'm' => multi_line = true,
            'u' => {}
            other => return Err(format!("unknown flag `{other}`")),
        }
    }
    let re = RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .multi_line(multi_line)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(RegExp { re, has_g })
}

impl RegExp {
    /// Whether the pattern matches anywhere in `input`.
    pub fn test(&self, input: &str) -> bool {
        self.re.is_match(input)
    }

    /// The first match in `input`, or `None` when there is none.
    pub fn find(&self, input: &str) -> Option<Match> {
        self.re.captures(input).as_ref().map(match_of)
    }

    /// Every match in `input`, in order.
    pub fn find_all(&self, input: &str) -> Vec<Match> {
        self.re.captures_iter(input).map(|c| match_of(&c)).collect()
    }

    /// Substitute matches with a `$1`/`${name}` template — all matches when
    /// `g` is set, the first otherwise.
    pub fn replace(&self, input: &str, template: &str) -> String {
        if self.has_g {
            self.re.replace_all(input, template).into_owned()
        } else {
            self.re.replace(input, template).into_owned()
        }
    }

    /// Substitute matches with a callback — all matches when `g` is set, the
    /// first otherwise. The caller (the interpreter) supplies the callback so
    /// an xulo closure can build each replacement; its error type flows
    /// through, so a failing callback stops the substitution.
    pub fn replace_with<E, F: FnMut(&Match) -> Result<String, E>>(
        &self,
        input: &str,
        mut replacer: F,
    ) -> Result<String, E> {
        let limit = if self.has_g { usize::MAX } else { 1 };
        let mut out = String::new();
        let mut last = 0;
        for caps in self.re.captures_iter(input).take(limit) {
            let whole = caps.get(0).expect("captures_iter yields group 0");
            out.push_str(&input[last..whole.start()]);
            out.push_str(&replacer(&match_of(&caps))?);
            last = whole.end();
        }
        out.push_str(&input[last..]);
        Ok(out)
    }
}
