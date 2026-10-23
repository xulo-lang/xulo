//! The `Path` namespace: pure string operations on `/`-separated paths,
//! companion to [`crate::fs`] (which takes raw path strings). Semantics use
//! POSIX separators (`/` only; `\` is an ordinary character), and trailing
//! slashes are ignored before components are taken — like Go's
//! `path/filepath`, so results are identical on every host. Every member is
//! total: a degenerate input (`""`, all slashes, a lone `..`) still has a
//! defined answer, so no member returns `Result`.

/// `Path::join(a, b)`: `b` appended to `a`. An absolute `b` (leading `/`)
/// replaces `a` entirely, mirroring `Path::join` / `os.path.join`.
pub fn join(a: &str, b: &str) -> String {
    // An absolute `b` replaces `a`; an empty `a` contributes no separator.
    if b.starts_with('/') || a.is_empty() {
        b.to_string()
    } else if a.ends_with('/') {
        format!("{a}{b}")
    } else {
        format!("{a}/{b}")
    }
}

/// `Path::dirname(path)`: the parent directory — everything before the last
/// `/`, with trailing slashes ignored; `""` when there is no parent (`"a"` →
/// `""`), `"/"` for a name directly under the root, and `""` for `""`.
pub fn dirname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        // "", "/", "///": no parent component; the root is its own parent.
        return if path.is_empty() {
            String::new()
        } else {
            "/".to_string()
        };
    }
    match trimmed.rfind('/') {
        None => String::new(),
        Some(0) => "/".to_string(),
        Some(i) => trimmed[..i].to_string(),
    }
}

/// `Path::basename(path)`: the final component with trailing slashes
/// ignored; `""` when the path names no component (`""`, `"/"`).
pub fn basename(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        None => trimmed.to_string(),
        Some(i) => trimmed[i + 1..].to_string(),
    }
}

/// `Path::ext(path)`: the extension of the final component **including** the
/// dot (`".txt"`, `".gz"`), or `""` when there is none. A dot at the start of
/// the name (`".bashrc"`) and a trailing dot (`"file."`) are not extensions.
pub fn ext(path: &str) -> String {
    let name = basename(path);
    match name.rfind('.') {
        // `i > 0`: a leading dot names a hidden file, not an extension.
        // `i + 1 < len`: a trailing dot carries no extension text.
        Some(i) if i > 0 && i + 1 < name.len() => name[i..].to_string(),
        _ => String::new(),
    }
}

/// `Path::normalize(path)`: collapse duplicate slashes, `"."` components,
/// and `".."` against the component before them (dropping `".."` at the root
/// of an absolute path, keeping leading `".."` of a relative one). A result
/// with nothing left is `"."` (or `"/"` for the root); trailing slashes are
/// removed.
pub fn normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut stack: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => match stack.last() {
                Some(&last) if last != ".." => {
                    stack.pop();
                }
                // A relative path keeps unmatched `".."` (it may exist once
                // directories appear); an absolute one is pinned at `/`.
                Some(_) if !absolute => stack.push(".."),
                None if !absolute => stack.push(".."),
                _ => {}
            },
            component => stack.push(component),
        }
    }
    let joined = stack.join("/");
    if absolute {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}
