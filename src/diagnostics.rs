use crate::error::{ErrorKind, XuloError};

const CYAN: &str = "\x1b[36m";
const MAGENTA: &str = "\x1b[35m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

thread_local! {
    static COLORS: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Enable or disable ANSI color. Honors `NO_COLOR` when `enabled` is true.
pub fn use_color(enabled: bool) {
    let on = enabled && std::env::var_os("NO_COLOR").is_none();
    COLORS.with(|c| c.set(on));
}

fn paint(code: &str, text: &str) -> String {
    if COLORS.with(|c| c.get()) {
        format!("{code}{text}{RESET}")
    } else {
        text.to_string()
    }
}

/// Render a pretty, source-annotated error report.
///
/// ```text
/// error[E0001] semantic: undefined variable 'message'
///  --> main.xulo:3:3
///   │
/// 3 │   print(message)
///   │         ^^^^^^^
/// ```
pub fn render(err: &XuloError, source: Option<&str>) -> String {
    let mut out = String::new();
    let is_warning = err.kind == ErrorKind::Warning;
    let severity = if is_warning { "warning" } else { "error" };
    let mut header = format!(
        "{}[{}]",
        paint(BOLD, severity),
        paint(CYAN, err.kind.code())
    );
    if !is_warning {
        header.push_str(&format!(" {}", err.kind.label()));
    }
    header.push_str(&format!(": {}", err.message));
    out.push_str(&header);
    out.push('\n');

    let Some(span) = &err.span else {
        if let Some(f) = &err.file {
            out.push_str(&format!(" ==> {}\n", paint(CYAN, &f.display().to_string())));
        }
        return out;
    };

    let Some((line_no, column, line)) = source.map(|src| locate(src, span.start)) else {
        if let Some(f) = &err.file {
            out.push_str(&format!(" ==> {}\n", paint(CYAN, &f.display().to_string())));
        }
        return out;
    };

    let path = err
        .file
        .as_ref()
        .map(|f| f.display().to_string())
        .unwrap_or_else(|| "<input>".to_string());
    out.push_str(&format!(
        " {} {}:{}:{}\n\n",
        paint(CYAN, "-->"),
        path,
        line_no,
        column
    ));

    let width = line_no.to_string().len();
    let gutter = |s: &str| {
        format!(
            "{}{:>width$} {}│{} ",
            paint(BOLD, " "),
            s,
            paint(MAGENTA, " "),
            RESET,
            width = width
        )
    };

    let caret_col = column.saturating_sub(1);
    let trimmed = line.trim_end();

    out.push_str(&gutter(""));
    out.push('\n');
    out.push_str(&gutter(&line_no.to_string()));
    out.push_str(trimmed);
    out.push('\n');
    out.push_str(&gutter(""));
    let visible = trimmed.len().saturating_sub(caret_col);
    out.push_str(&" ".repeat(caret_col));
    out.push_str(&paint(CYAN, &"^".repeat(visible.max(1))));
    out.push('\n');

    out
}

/// Locate the 1-based line number, 1-based column, and the line text for a
/// byte offset in `src`.
pub fn locate(src: &str, offset: usize) -> (usize, usize, &str) {
    let mut line_no = 1usize;
    let mut line_start = 0usize;
    for (i, c) in src.char_indices() {
        if i >= offset {
            let line_end = src[line_start..]
                .find('\n')
                .map(|p| line_start + p)
                .unwrap_or(src.len());
            return (line_no, offset - line_start + 1, &src[line_start..line_end]);
        }
        if c == '\n' {
            line_no += 1;
            line_start = i + 1;
        }
    }
    (line_no, offset - line_start + 1, &src[line_start..])
}
