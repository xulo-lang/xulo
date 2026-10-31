//! `std::fmt` (spec `std-spec/modules/fmt.md`): one member, `format`, that
//! assembles formatted text from a template of literal text and `%` directives.
//!
//! The engine is pure: parse the template into pieces, check the directive
//! count against the argument list, then render each directive from one
//! [`FmtArg`]. Every malformed template and every mismatched operand is an
//! `Err` — the interpreter turns it into a `RunError`, the compiled path's
//! `xulo_fmt_format` into a runtime failure (spec `memory-and-runtime.md`,
//! § 5.3). Nothing here touches I/O or state.

/// One formatted argument. The interpreter maps its `Value` onto this enum;
/// the compiled path maps the (payload, tag) pair. `Rendered` carries the
/// pre-rendered `str` text of a container (or any value the verbs treat as
/// its `str` rendering), so the engine itself never sees the runtime's
/// recursive value structures.
#[derive(Debug, Clone)]
pub enum FmtArg {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Null,
    Rendered(String),
}

/// A parsed `%` directive: verb, flags, width, precision.
#[derive(Debug, Clone, Copy)]
struct Directive {
    verb: char,
    dash: bool,
    plus: bool,
    zero: bool,
    space: bool,
    alt: bool,
    width: Option<usize>,
    precision: Option<usize>,
}

/// One piece of a parsed template: literal text or a directive.
enum Piece {
    Lit(String),
    Dir(Directive),
}

/// The verbs that take a numeric operand (where `+`, ` `, and `0` are legal).
fn is_numeric_verb(verb: char) -> bool {
    matches!(verb, 'd' | 'x' | 'X' | 'o' | 'b' | 'f' | 'e' | 'E' | 'g' | 'G')
}

/// The verbs that take `#` (alternate form).
fn is_alt_verb(verb: char) -> bool {
    matches!(verb, 'x' | 'X' | 'o' | 'b')
}

fn is_verb(verb: char) -> bool {
    matches!(
        verb,
        'v' | 's' | 'q' | 't' | 'c' | 'd' | 'x' | 'X' | 'o' | 'b' | 'f' | 'e' | 'E' | 'g' | 'G'
    )
}

fn err(message: impl Into<String>) -> String {
    format!("`std::fmt::format` {}", message.into())
}

/// Widths above this are refused rather than allocated: the same ceiling
/// `string.repeat`/`pad_*` carry in the runtime (spec `memory-and-runtime.md`).
const MAX_WIDTH: usize = 1 << 20;

/// The precision domains: `%f`/`%e`/`%E` count digits after the point,
/// `%g`/`%G` significant digits (spec `modules/fmt.md`, "Failure model").
fn precision_domain(verb: char) -> Option<(usize, usize)> {
    match verb {
        'f' | 'e' | 'E' => Some((0, 15)),
        'g' | 'G' => Some((1, 17)),
        _ => None,
    }
}

/// Parse the template into pieces. Validates every directive's shape, flags,
/// and precision domain; the directive count is checked by [`format`].
fn parse(template: &str) -> Result<Vec<Piece>, String> {
    let chars: Vec<char> = template.chars().collect();
    let mut pieces: Vec<Piece> = Vec::new();
    let mut lit = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '%' {
            lit.push(chars[i]);
            i += 1;
            continue;
        }
        i += 1; // '%'
        if i < chars.len() && chars[i] == '%' {
            lit.push('%');
            i += 1;
            continue;
        }
        let mut dash = false;
        let mut plus = false;
        let mut zero = false;
        let mut space = false;
        let mut alt = false;
        while i < chars.len() {
            let flag = match chars[i] {
                '-' => &mut dash,
                '+' => &mut plus,
                '0' => &mut zero,
                ' ' => &mut space,
                '#' => &mut alt,
                _ => break,
            };
            if *flag {
                return Err(err(format!("repeated flag `{}` in directive", chars[i])));
            }
            *flag = true;
            i += 1;
        }
        let width = parse_number(&mut i, &chars, "width")?;
        let mut precision = None;
        if i < chars.len() && chars[i] == '.' {
            i += 1;
            let Some(p) = parse_number(&mut i, &chars, "precision")? else {
                return Err(err("`.` must be followed by precision digits"));
            };
            precision = Some(p);
        }
        let Some(&verb) = chars.get(i) else {
            return Err(err("incomplete directive: `%` with no verb"));
        };
        if verb == '%' {
            return Err(err("`%%` takes no flags, width, or precision"));
        }
        if !is_verb(verb) {
            return Err(err(format!("unknown verb `{verb}`")));
        }
        i += 1;
        if (plus || space) && !is_numeric_verb(verb) {
            let flag = if plus { '+' } else { ' ' };
            return Err(err(format!("flag `{flag}` is not valid for verb `{verb}`")));
        }
        if zero && !is_numeric_verb(verb) {
            return Err(err(format!("flag `0` is not valid for verb `{verb}`")));
        }
        if alt && !is_alt_verb(verb) {
            return Err(err(format!("flag `#` is not valid for verb `{verb}`")));
        }
        if let Some(p) = precision {
            if let Some((lo, hi)) = precision_domain(verb) {
                if !(lo..=hi).contains(&p) {
                    return Err(err(format!(
                        "precision out of range for verb `{verb}`: {lo} to {hi}"
                    )));
                }
            } else if verb != 's' && verb != 'q' {
                return Err(err(format!("precision is not valid for verb `{verb}`")));
            }
        }
        if !lit.is_empty() {
            pieces.push(Piece::Lit(std::mem::take(&mut lit)));
        }
        pieces.push(Piece::Dir(Directive {
            verb,
            dash,
            plus,
            zero,
            space,
            alt,
            width,
            precision,
        }));
    }
    if !lit.is_empty() {
        pieces.push(Piece::Lit(lit));
    }
    Ok(pieces)
}

/// Consume a run of digits as a number, or `None` when the first character is
/// not a digit. Overwide values are refused rather than wrapped.
fn parse_number(i: &mut usize, chars: &[char], what: &str) -> Result<Option<usize>, String> {
    let start = *i;
    while *i < chars.len() && chars[*i].is_ascii_digit() {
        *i += 1;
    }
    if start == *i {
        return Ok(None);
    }
    let text: String = chars[start..*i].iter().collect();
    let value: usize = text
        .parse()
        .map_err(|_| err(format!("{what} `{text}` is too large")))?;
    if value > MAX_WIDTH {
        return Err(err(format!("{what} `{text}` is too large")));
    }
    Ok(Some(value))
}

/// The operand's short kind name for error messages.
fn arg_kind(arg: &FmtArg) -> &'static str {
    match arg {
        FmtArg::Str(_) => "a string",
        FmtArg::Int(_) => "an int",
        FmtArg::Float(_) => "a float",
        FmtArg::Bool(_) => "a bool",
        FmtArg::Null => "null",
        FmtArg::Rendered(_) => "a rendered value",
    }
}

/// The operand as a string for the `%v`/`%s` verbs (the `str` rendering).
fn text_of(arg: &FmtArg) -> String {
    match arg {
        FmtArg::Str(s) => s.clone(),
        FmtArg::Int(n) => n.to_string(),
        FmtArg::Float(f) => crate::format_number(*f),
        FmtArg::Bool(b) => b.to_string(),
        FmtArg::Null => "null".to_string(),
        FmtArg::Rendered(s) => s.clone(),
    }
}

/// The operand as an integer for the integer verbs. A whole float is its
/// exact integer value (the runtime has one number model: `2` and `2.0` are
/// the same value), anything else is a mismatch.
fn int_of(arg: &FmtArg) -> Option<i64> {
    match arg {
        FmtArg::Int(n) => Some(*n),
        FmtArg::Float(f) => {
            if f.fract() == 0.0 && f.is_finite() && (i64::MIN as f64..=i64::MAX as f64).contains(f) {
                Some(*f as i64)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The operand as a float for the floating-point verbs (an int renders as
/// its exact float value).
fn float_of(arg: &FmtArg) -> Option<f64> {
    match arg {
        FmtArg::Int(n) => Some(*n as f64),
        FmtArg::Float(f) => Some(*f),
        _ => None,
    }
}

/// Truncate `text` to at most `n` Unicode scalars.
fn cut(text: String, n: usize) -> String {
    text.chars().take(n).collect()
}

/// The `%q` quoting: a Xulo string literal's escapes (spec `grammar.md`,
/// `EscapeSeq`).
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if (c as u32) < 0x20 || c as u32 == 0x7F => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The `%e`/`%E` exponent spelling: sign, at least two digits.
fn exponent_text(exp: i32) -> String {
    if exp < 0 {
        format!("-{:02}", exp.unsigned_abs())
    } else {
        format!("+{:02}", exp)
    }
}

/// Round the decimal digit string of `abs` (as `{:e}` writes it) to `sig`
/// significant digits, halfway away from zero. Returns the significant digits
/// and the decimal exponent of the leading digit: the value is
/// `digits[0].digits[1..] × 10^exp`. Zero yields `([0], 0)`.
fn round_sig(abs: f64, sig: usize) -> (Vec<u8>, i32) {
    if abs == 0.0 {
        return (vec![0], 0);
    }
    let e = format!("{abs:e}");
    let (mantissa, exp) = e.split_once('e').expect("`{:e}` always carries `e`");
    let exp: i32 = exp.parse().unwrap_or(0);
    let mut digits: Vec<u8> = mantissa
        .chars()
        .filter(|c| c.is_ascii_digit())
        .map(|c| (c as u8) - b'0')
        .collect();
    if digits.len() > sig {
        let mut keep: Vec<u8> = digits[..sig].to_vec();
        if digits[sig] >= 5 {
            // Increment the kept prefix, half away from zero; a full carry
            // (999 → 1000) shortens back to `sig` digits and bumps the
            // exponent.
            let mut k = sig;
            loop {
                if k == 0 {
                    keep.insert(0, 1);
                    break;
                }
                k -= 1;
                if keep[k] == 9 {
                    keep[k] = 0;
                } else {
                    keep[k] += 1;
                    break;
                }
            }
            if keep.len() > sig {
                keep.truncate(sig);
                return (keep, exp + 1);
            }
        }
        digits = keep;
    }
    (digits, exp)
}

/// Pad a significant-digit string with trailing zeros to exactly `n` digits.
fn pad_digits(mut digits: Vec<u8>, n: usize) -> Vec<u8> {
    while digits.len() < n {
        digits.push(0);
    }
    digits
}

/// Drop trailing zeros from a significant-digit string (at least one digit
/// remains) — `%g` prints no trailing zeros in either form.
fn trim_digits(mut digits: Vec<u8>) -> Vec<u8> {
    while digits.len() > 1 && digits.last() == Some(&0) {
        digits.pop();
    }
    digits
}

/// The fixed-point form of a significant-digit string: point after `exp + 1`
/// digits, negative exponents spelled `0.00…digits`.
fn fixed_form(digits: &[u8], exp: i32) -> String {
    let mut out = String::new();
    if exp >= 0 {
        let int_len = exp as usize + 1;
        if digits.len() >= int_len {
            for d in &digits[..int_len] {
                out.push((b'0' + d) as char);
            }
            let frac = &digits[int_len..];
            if !frac.is_empty() {
                out.push('.');
                for d in frac {
                    out.push((b'0' + d) as char);
                }
            }
        } else {
            for d in digits {
                out.push((b'0' + d) as char);
            }
            for _ in 0..(int_len - digits.len()) {
                out.push('0');
            }
        }
    } else {
        out.push_str("0.");
        for _ in 0..(-exp - 1) {
            out.push('0');
        }
        for d in digits {
            out.push((b'0' + d) as char);
        }
    }
    out
}

/// Drop trailing zeros (and a point left bare) from a fixed form.
fn trim_frac(text: String) -> String {
    if !text.contains('.') {
        return text;
    }
    let trimmed = text.trim_end_matches('0');
    match trimmed.strip_suffix('.') {
        Some(head) => head.to_string(),
        None => trimmed.to_string(),
    }
}

/// The scientific form: one digit, optional fraction digits, `e`/`E` and the
/// signed exponent.
fn sci_form(digits: &[u8], exp: i32, upper: bool) -> String {
    let mut out = String::new();
    out.push((b'0' + digits[0]) as char);
    let frac = if digits.len() > 1 { &digits[1..] } else { &[][..] };
    if !frac.is_empty() {
        out.push('.');
        for d in frac {
            out.push((b'0' + d) as char);
        }
    }
    out.push(if upper { 'E' } else { 'e' });
    out.push_str(&exponent_text(exp));
    out
}

/// Assemble sign, `#` prefix, and body under the width rules (sign-aware
/// zero padding; `0` is ignored when `-` is given).
fn assemble(dir: &Directive, verb: char, sign: &str, alt: &str, body: &str) -> String {
    let body_len = sign.chars().count() + alt.chars().count() + body.chars().count();
    let width = dir.width.unwrap_or(0);
    if width <= body_len {
        return format!("{sign}{alt}{body}");
    }
    let fill = width - body_len;
    if dir.dash {
        format!("{sign}{alt}{body}{}", " ".repeat(fill))
    } else if dir.zero && is_numeric_verb(verb) {
        format!("{sign}{alt}{}{body}", "0".repeat(fill))
    } else {
        format!("{}{sign}{alt}{body}", " ".repeat(fill))
    }
}

/// The sign of a numeric operand: `-` for negatives (a negative zero keeps
/// its sign), `+` under the `+` flag, a space under the ` ` flag, else none.
fn sign_of(dir: &Directive, negative: bool) -> &'static str {
    if negative {
        "-"
    } else if dir.plus {
        "+"
    } else if dir.space {
        " "
    } else {
        ""
    }
}

/// Render one directive against its operand.
fn render_dir(dir: &Directive, arg: &FmtArg) -> Result<String, String> {
    let verb = dir.verb;
    let mut negative = false;
    let mut alt = "";
    let body;
    match verb {
        'v' => {
            body = text_of(arg);
        }
        's' => {
            let text = text_of(arg);
            body = match dir.precision {
                Some(p) => cut(text, p),
                None => text,
            };
        }
        'q' => {
            let FmtArg::Str(text) = arg else {
                return Err(err(format!("`%q` takes a string, found {}", arg_kind(arg))));
            };
            let text = match dir.precision {
                Some(p) => cut(text.clone(), p),
                None => text.clone(),
            };
            body = quote(&text);
        }
        't' => {
            let FmtArg::Bool(b) = arg else {
                return Err(err(format!(
                    "`%t` takes a bool value, found {}",
                    arg_kind(arg)
                )));
            };
            body = b.to_string();
        }
        'c' => {
            let Some(n) = int_of(arg) else {
                return Err(err(format!(
                    "`%c` takes an int value, found {}",
                    arg_kind(arg)
                )));
            };
            let Ok(u) = u32::try_from(n) else {
                return Err(err(format!(
                    "`%c` scalar out of range: {n} is not in 0 …= 0x10FFFF"
                )));
            };
            if u > 0x10FFFF {
                return Err(err(format!(
                    "`%c` scalar out of range: {n} is not in 0 …= 0x10FFFF"
                )));
            }
            let Some(c) = char::from_u32(u) else {
                return Err(err(format!("`%c` scalar out of range: {n} is a surrogate")));
            };
            body = c.to_string();
        }
        'd' | 'x' | 'X' | 'o' | 'b' => {
            let Some(n) = int_of(arg) else {
                return Err(err(format!(
                    "`%{verb}` takes an int value, found {}",
                    arg_kind(arg)
                )));
            };
            negative = n < 0;
            let mag = n.unsigned_abs();
            body = match verb {
                'd' => format!("{mag}"),
                'x' => format!("{mag:x}"),
                'X' => format!("{mag:X}"),
                'o' => format!("{mag:o}"),
                _ => format!("{mag:b}"),
            };
            alt = if dir.alt {
                match verb {
                    'x' => "0x",
                    'X' => "0X",
                    'o' => "0o",
                    'b' => "0b",
                    _ => "",
                }
            } else {
                ""
            };
        }
        'f' | 'e' | 'E' | 'g' | 'G' => {
            let Some(v) = float_of(arg) else {
                return Err(err(format!(
                    "`%{verb}` takes a number value, found {}",
                    arg_kind(arg)
                )));
            };
            // The three special values ignore precision and the `+`/` `/`#`/`0`
            // flags; width and `-` still place them.
            if v.is_nan() || v.is_infinite() {
                let text = crate::format_number(v);
                let width = dir.width.unwrap_or(0);
                let len = text.chars().count();
                if width <= len {
                    return Ok(text);
                }
                let fill = width - len;
                return Ok(if dir.dash {
                    format!("{text}{}", " ".repeat(fill))
                } else {
                    format!("{}{text}", " ".repeat(fill))
                });
            }
            negative = v.is_sign_negative();
            let abs = v.abs();
            body = match verb {
                'f' => {
                    let p = dir.precision.unwrap_or(6);
                    crate::math::to_fixed(abs, p)
                }
                'e' | 'E' => {
                    let p = dir.precision.unwrap_or(6);
                    let (digits, exp) = round_sig(abs, p + 1);
                    sci_form(&pad_digits(digits, p + 1), exp, verb == 'E')
                }
                _ => {
                    let upper = verb == 'G';
                    match dir.precision {
                        Some(p) => {
                            let (digits, exp) = round_sig(abs, p);
                            let digits = trim_digits(digits);
                            if !(-4..p as i32).contains(&exp) {
                                sci_form(&digits, exp, upper)
                            } else {
                                trim_frac(fixed_form(&digits, exp))
                            }
                        }
                        None => {
                            // Shortest digit string that reads back: `{:e}`'s
                            // mantissa is the shortest round-trip, already.
                            let (digits, exp) = round_sig(abs, 17);
                            if !(-4..6).contains(&exp) {
                                sci_form(&digits, exp, upper)
                            } else {
                                trim_frac(fixed_form(&digits, exp))
                            }
                        }
                    }
                }
            };
        }
        _ => unreachable!("verb checked by the parser"),
    }
    let sign = sign_of(dir, negative);
    Ok(assemble(dir, verb, sign, alt, &body))
}

/// `std::fmt::format(template, args)`: the template's literal text with each
/// directive replaced by its formatted argument (spec `std-spec/modules/
/// fmt.md`). A malformed template, a directive-count mismatch, or an operand
/// that does not suit its verb is an `Err` — a runtime failure.
pub fn format(template: &str, args: &[FmtArg]) -> Result<String, String> {
    let pieces = parse(template)?;
    let directives = pieces
        .iter()
        .filter(|p| matches!(p, Piece::Dir(_)))
        .count();
    if directives != args.len() {
        return Err(err(format!(
            "expects {directives} argument{}, found {}",
            if directives == 1 { "" } else { "s" },
            args.len()
        )));
    }
    let mut out = String::new();
    let mut next = 0;
    for piece in pieces {
        match piece {
            Piece::Lit(text) => out.push_str(&text),
            Piece::Dir(dir) => {
                let arg = &args[next];
                next += 1;
                out.push_str(&render_dir(&dir, arg)?);
            }
        }
    }
    Ok(out)
}
