//! Pre-parse bracket-nesting scan.
//!
//! pest's generated parser is plain recursive descent, so deeply nested
//! input can overflow the stack before the grammar has any chance to reject
//! it. Instead of relying on the OS stack, `check` walks the raw source once
//! — skipping strings, templates, and comments — and reports the byte offset
//! where the bracket nesting first exceeds the limit. `parse_program` runs it
//! before handing the source to pest, so hostile input gets a diagnostic
//! ("nesting is too deep") instead of a crash.

/// Where the scanner is while walking the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Ordinary source: brackets count, comments/strings/templates start.
    Code,
    /// Inside `// …` until the next line terminator.
    LineComment,
    /// Inside `/* … */` (non-nesting, per spec).
    BlockComment,
    /// Inside a quoted string of the given quote character.
    Str(char),
    /// Inside a backtick template literal (text part).
    Template,
    /// Inside `${ … }` of a template; carries the bracket depth at entry so
    /// the matching `}` can be recognized.
    Interp(usize),
}

/// Scan `source`; return the byte offset of the first opener that pushes the
/// bracket nesting past `limit`, or `None` when the source stays within it.
pub fn first_too_deep(source: &str, limit: usize) -> Option<usize> {
    let mut stack: Vec<Mode> = vec![Mode::Code];
    let mut depth: usize = 0;
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        // Work on bytes: every interesting character here is ASCII, and any
        // multi-byte UTF-8 sequence only ever appears as opaque content.
        match *stack.last().unwrap() {
            Mode::Code => match c {
                '/' if bytes.get(i + 1) == Some(&b'/') => {
                    stack.push(Mode::LineComment);
                    i += 2;
                }
                '/' if bytes.get(i + 1) == Some(&b'*') => {
                    stack.push(Mode::BlockComment);
                    i += 2;
                }
                '"' | '\'' => {
                    stack.push(Mode::Str(c));
                    i += 1;
                }
                '`' => {
                    stack.push(Mode::Template);
                    i += 1;
                }
                '(' | '[' | '{' => {
                    depth += 1;
                    if depth > limit {
                        return Some(i);
                    }
                    i += 1;
                }
                ')' | ']' | '}' => {
                    depth = depth.saturating_sub(1);
                    i += 1;
                }
                _ => i += 1,
            },
            Mode::LineComment => {
                if c == '\n' {
                    stack.pop();
                }
                i += 1;
            }
            Mode::BlockComment => {
                if c == '*' && bytes.get(i + 1) == Some(&b'/') {
                    stack.pop();
                    i += 2;
                } else {
                    i += 1;
                }
            }
            Mode::Str(q) => {
                if c == '\\' {
                    i += 2;
                } else if c == q {
                    stack.pop();
                    i += 1;
                } else {
                    i += 1;
                }
            }
            Mode::Template => match c {
                '\\' => i += 2,
                '`' => {
                    stack.pop();
                    i += 1;
                }
                '$' if bytes.get(i + 1) == Some(&b'{') => {
                    stack.push(Mode::Interp(depth));
                    i += 2;
                }
                _ => i += 1,
            },
            Mode::Interp(_) => {
                // Interpolation contents scan like ordinary code; the mode
                // entry is only consulted when its closing `}` shows up.
                // Re-dispatch through Code by temporarily swapping the top.
                let base = match stack.last() {
                    Some(Mode::Interp(b)) => *b,
                    _ => unreachable!(),
                };
                debug_assert!(base <= depth);
                // Inline the Code arm without popping the Interp marker:
                // treat this position as code but keep the mode.
                match c {
                    '/' if bytes.get(i + 1) == Some(&b'/') => {
                        stack.push(Mode::LineComment);
                        i += 2;
                    }
                    '/' if bytes.get(i + 1) == Some(&b'*') => {
                        stack.push(Mode::BlockComment);
                        i += 2;
                    }
                    '"' | '\'' => {
                        stack.push(Mode::Str(c));
                        i += 1;
                    }
                    '`' => {
                        stack.push(Mode::Template);
                        i += 1;
                    }
                    '(' | '[' | '{' => {
                        depth += 1;
                        if depth > limit {
                            return Some(i);
                        }
                        i += 1;
                    }
                    '}' if depth == base => {
                        stack.pop();
                        i += 1;
                    }
                    '}' => {
                        depth = depth.saturating_sub(1);
                        i += 1;
                    }
                    _ => i += 1,
                }
            }
        }
    }
    None
}
