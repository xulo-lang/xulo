/// Format a number the way `print`/`str` show it: `3.0` -> `"3"`, `NaN` ->
/// `"NaN"`, `Infinity` -> `"Infinity"`, `-Infinity` -> `"-Infinity"`, and
/// `-0` -> `"0"` (spec `builtins/intrinsic-functions.md`, Output).
pub fn format_number(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    if n == 0.0 && n.is_sign_negative() {
        return "0".into();
    }
    format!("{n}")
}
