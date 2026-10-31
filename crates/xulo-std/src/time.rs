//! The `Time` namespace (spec `builtins/intrinsic-functions.md`): wall-clock
//! and monotonic readings shared by the interpreter and the compiled path's
//! [`crate::ffi`] entry points. `Time::sleep` is not here — it needs the
//! interpreter's task scheduler.

/// `Time::now`: milliseconds since the Unix epoch (wall clock).
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `Time::nowNanos`: nanoseconds since the Unix epoch (wall clock).
pub fn now_nanos() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// The process's monotonic baseline: `Instant::now()` at first use, so every
/// reading is a difference from a fixed origin that never runs backwards.
fn monotonic_start() -> std::time::Instant {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *START.get_or_init(std::time::Instant::now)
}

/// `Time::monotonic`: milliseconds on the monotonic clock.
pub fn monotonic_millis() -> i64 {
    monotonic_start().elapsed().as_millis() as i64
}

/// `Time::monotonicNanos`: nanoseconds on the monotonic clock.
pub fn monotonic_nanos() -> i64 {
    monotonic_start().elapsed().as_nanos() as i64
}

// ──────────────────────────────────────────────────────────────────
//  The calendar (spec `modules/time.md`, "Calendar"): UTC alone — no host
//  time zone, no DST, no locale — over the timestamp domain
//  `0 …= 253402300799999` ms (`1970-01-01T00:00:00.000Z` …
//  `9999-12-31T23:59:59.999Z`), leap seconds not represented.
// ──────────────────────────────────────────────────────────────────

/// The calendar's timestamp domain: `1970-01-01T00:00:00.000Z` through
/// `9999-12-31T23:59:59.999Z`.
pub const MAX_TIME_MS: u64 = 253_402_300_799_999;

/// One `Time::parts` record: the UTC date and time fields of a timestamp.
#[derive(Clone, Copy, Debug)]
pub struct TimeParts {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub minute: i64,
    pub second: i64,
    pub millisecond: i64,
}

/// Days since the Unix epoch for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The inverse of [`days_from_civil`].
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Leap years are the ordinary Gregorian rule.
fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// The day count of a month.
fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// `Time::parts`: the UTC date and time fields of a timestamp. Total for
/// every `u64`, the calendar's domain included — a timestamp `format`
/// refuses still decomposes.
pub fn parts(ms: u64) -> TimeParts {
    let days = (ms / 86_400_000) as i64;
    let tod = ms % 86_400_000;
    let (year, month, day) = civil_from_days(days);
    TimeParts {
        year,
        month,
        day,
        hour: (tod / 3_600_000) as i64,
        minute: (tod / 60_000 % 60) as i64,
        second: (tod / 1_000 % 60) as i64,
        millisecond: (tod % 1_000) as i64,
    }
}

/// `Time::weekday`: the day of the week — `0` Sunday … `6` Saturday. Total
/// for every `u64`; the epoch itself was a Thursday (`weekday(0) == 4`).
pub fn weekday(ms: u64) -> i64 {
    ((ms / 86_400_000) as i64 + 4).rem_euclid(7)
}

/// `Time::format`: the timestamp as ISO 8601 UTC — one fixed layout, four-
/// digit year and exactly three fractional digits. A `ms` past the domain
/// is a runtime failure (`Err` here; the caller stops the program).
pub fn format(ms: u64) -> Result<String, String> {
    if ms > MAX_TIME_MS {
        return Err(format!(
            "`std::time::format` ms must be at most {MAX_TIME_MS} (9999-12-31T23:59:59.999Z), found {ms}"
        ));
    }
    let p = parts(ms);
    Ok(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        p.year, p.month, p.day, p.hour, p.minute, p.second, p.millisecond
    ))
}

/// `Time::fromParts`: the timestamp those UTC fields denote. The fields
/// must name a real instant inside the domain — any other field set is a
/// runtime failure (`Err` here; the caller stops the program).
pub fn from_parts(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    millisecond: i64,
) -> Result<u64, String> {
    let bad = |why: &str| {
        Err(format!(
            "`std::time::fromParts` fields must name a real instant in the calendar domain: {why} ({year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millisecond:03})"
        ))
    };
    if !(1..=12).contains(&month) {
        return bad("month outside 1..=12");
    }
    if day < 1 || day > days_in_month(year, month) {
        return bad("day outside the month");
    }
    if !(0..=23).contains(&hour) {
        return bad("hour outside 0..=23");
    }
    if !(0..=59).contains(&minute) {
        return bad("minute outside 0..=59");
    }
    if !(0..=59).contains(&second) {
        return bad("second outside 0..=59");
    }
    if !(0..=999).contains(&millisecond) {
        return bad("millisecond outside 0..=999");
    }
    let days = days_from_civil(year, month, day);
    if days < 0 {
        return bad("before the Unix epoch");
    }
    let ms = days * 86_400_000
        + hour * 3_600_000
        + minute * 60_000
        + second * 1_000
        + millisecond;
    if ms < 0 || ms as u64 > MAX_TIME_MS {
        return bad("past 9999-12-31T23:59:59.999Z");
    }
    Ok(ms as u64)
}

/// `Time::parse`: the timestamp an ISO 8601 UTC string denotes. The layout
/// `format` emits, with `T` or a single space as separator and a fraction
/// of 1 to 9 digits (truncated to the millisecond); `Z` is required, field
/// ranges are checked against the calendar, and everything else —
/// malformed text, before the epoch, beyond `9999` — is `Err`.
pub fn parse(text: &str) -> Result<u64, String> {
    let malformed = || Err(format!("malformed ISO 8601 UTC text: `{text}`"));
    let b = text.as_bytes();
    // `YYYY-MM-DDTHH:MM:SS` is the fixed head; `sep` is `T` or one space.
    if b.len() < 20 {
        return malformed();
    }
    let digits = |r: std::ops::Range<usize>| -> Option<i64> {
        let s = text.get(r)?;
        if !s.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    };
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        digits(0..4),
        digits(5..7),
        digits(8..10),
        digits(11..13),
        digits(14..16),
        digits(17..19),
    ) else {
        return malformed();
    };
    if b[4] != b'-' || b[7] != b'-' || b[13] != b':' || b[16] != b':' {
        return malformed();
    }
    if b[10] != b'T' && b[10] != b' ' {
        return malformed();
    }
    // The fraction is 1 to 9 digits, truncated to the millisecond; `Z` is
    // required — an offset such as `+08:00` is not part of this version.
    let mut frac_ms = 0i64;
    let mut end = 19;
    if b[end] == b'.' {
        let mut i = end + 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() && i - start < 9 {
            i += 1;
        }
        if i == start {
            return malformed();
        }
        // Truncate the fraction to millisecond precision.
        for (n, c) in text[start..i].bytes().enumerate().take(3) {
            frac_ms += (c - b'0') as i64 * [100, 10, 1][n];
        }
        end = i;
    }
    if end != b.len() - 1 || b[end] != b'Z' {
        return malformed();
    }
    if !(1970..=9999).contains(&year) {
        return Err(format!(
            "ISO 8601 UTC text is outside the calendar domain: `{text}`"
        ));
    }
    let instant = from_parts(year, month, day, hour, minute, second, frac_ms)
        .map_err(|_| format!("malformed ISO 8601 UTC text: `{text}`"))?;
    Ok(instant)
}
