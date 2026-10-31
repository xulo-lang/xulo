//! The `Time` namespace's calendar in isolation (spec
//! `std-spec/modules/time.md`, "Calendar"): UTC alone over the
//! `0 …= 253402300799999` ms domain, no host involved.

use xulo_std::time;

#[test]
fn format_emits_one_fixed_layout() {
    assert_eq!(
        time::format(0).expect("epoch"),
        "1970-01-01T00:00:00.000Z"
    );
    // The spec's own example: `fromParts(2026, 10, 26, 12)`.
    let noon = time::from_parts(2026, 10, 26, 12, 0, 0, 0).expect("in domain");
    assert_eq!(time::format(noon).expect("in domain"), "2026-10-26T12:00:00.000Z");
    // Exactly three fractional digits and the `Z` suffix.
    assert_eq!(
        time::format(1_761_480_000_123).expect("in domain"),
        "2025-10-26T12:00:00.123Z"
    );
}

#[test]
fn format_refuses_a_timestamp_past_the_domain() {
    assert!(time::format(time::MAX_TIME_MS).is_ok());
    let err = time::format(time::MAX_TIME_MS + 1).expect_err("past 9999");
    assert!(err.contains("9999-12-31T23:59:59.999Z"), "{err}");
}

#[test]
fn parse_inverts_format_over_the_whole_domain() {
    for ms in [
        0u64,
        1,
        999,
        86_400_000,
        1_761_480_000_123,
        time::MAX_TIME_MS,
    ] {
        let text = time::format(ms).expect("in domain");
        assert_eq!(time::parse(&text).expect("roundtrip"), ms, "{text}");
    }
}

#[test]
fn parse_accepts_the_two_relaxations() {
    let noon = time::from_parts(2026, 10, 26, 12, 0, 0, 0).expect("in domain");
    assert_eq!(
        time::parse("2026-10-26 12:00:00Z").expect("space separator"),
        noon
    );
    // A fraction of 1 to 9 digits, truncated to the millisecond.
    assert_eq!(
        time::parse("2026-10-26T12:00:00.7Z").expect("fraction"),
        noon + 700
    );
    assert_eq!(
        time::parse("2026-10-26T12:00:00.789123456Z").expect("fraction"),
        noon + 789
    );
}

#[test]
fn parse_rejects_everything_else_as_an_error() {
    for text in [
        "",
        "2026-10-26",
        "2026-10-26T12:00:00",         // no Z
        "2026-10-26T12:00:00+08:00",   // an offset is not in this version
        "2026-02-30T00:00:00Z",        // no such date
        "2026-10-26T23:59:60Z",        // leap seconds are not represented
        "2026-10-26T24:00:00Z",        // hour out of range
        "1969-12-31T23:59:59Z",        // before the Unix epoch
        "2026-10-26T12:00:00.Z",       // fraction with no digits
        "2026-10-26T12:00:00.abcdefghiZ",
        "2026x10-26T12:00:00Z",
    ] {
        let err = time::parse(text).expect_err(text);
        assert!(!err.is_empty(), "{text}");
    }
}

#[test]
fn parts_decomposes_even_past_the_domain() {
    let noon = time::from_parts(2026, 10, 26, 12, 0, 0, 0).expect("in domain");
    let p = time::parts(noon);
    assert_eq!(
        (p.year, p.month, p.day, p.hour, p.minute, p.second, p.millisecond),
        (2026, 10, 26, 12, 0, 0, 0)
    );
    // Total for every u64: a timestamp `format` refuses still decomposes.
    let p = time::parts(time::MAX_TIME_MS + 1);
    assert_eq!(
        (p.year, p.month, p.day, p.hour, p.minute, p.second, p.millisecond),
        (10000, 1, 1, 0, 0, 0, 0)
    );
}

#[test]
fn from_parts_inverts_parts_and_defaults_the_time_fields() {
    // `fromParts(2026, 10, 26)` is midnight UTC.
    let midnight = time::from_parts(2026, 10, 26, 0, 0, 0, 0).expect("in domain");
    assert_eq!(time::from_parts(2026, 10, 26, 0, 0, 0, 0).unwrap(), midnight);
    for ms in [0u64, 86_399_999, 1_761_480_000_123, time::MAX_TIME_MS] {
        let p = time::parts(ms);
        assert_eq!(
            time::from_parts(p.year, p.month, p.day, p.hour, p.minute, p.second, p.millisecond)
                .expect("roundtrip"),
            ms
        );
    }
}

#[test]
fn from_parts_rejects_fields_outside_the_calendar() {
    for (y, mo, d, h, mi, s, ms) in [
        (2026, 2, 30, 0, 0, 0, 0),   // the calendar has no such date
        (2026, 13, 1, 0, 0, 0, 0),   // month outside 1..=12
        (2026, 0, 1, 0, 0, 0, 0),
        (2026, 1, 0, 0, 0, 0, 0),    // day outside the month
        (2026, 4, 31, 0, 0, 0, 0),
        (2023, 2, 29, 0, 0, 0, 0),   // not a leap year
        (2026, 10, 26, 24, 0, 0, 0), // hour outside 0..=23
        (2026, 10, 26, 0, 60, 0, 0),
        (2026, 10, 26, 0, 0, 60, 0), // a leap second does not parse
        (2026, 10, 26, 0, 0, 0, 1000),
        (1969, 12, 31, 0, 0, 0, 0),  // before the Unix epoch
    ] {
        let err = time::from_parts(y, mo, d, h, mi, s, ms).expect_err("field set refused");
        assert!(err.contains("real instant"), "{err}");
    }
    // A leap day exists in a leap year.
    assert!(time::from_parts(2024, 2, 29, 0, 0, 0, 0).is_ok());
}

#[test]
fn weekday_is_the_day_count_reduced_by_seven() {
    assert_eq!(time::weekday(0), 4, "the epoch was a Thursday");
    // The spec's example: 2026-10-26 is a Monday.
    let noon = time::from_parts(2026, 10, 26, 12, 0, 0, 0).expect("in domain");
    assert_eq!(time::weekday(noon), 1);
    // Sunday … Saturday run 0 … 6.
    for (ms, want) in [(86_400_000u64, 5u64), (3 * 86_400_000, 0)] {
        assert_eq!(time::weekday(ms) as u64, want, "{ms}");
    }
}
