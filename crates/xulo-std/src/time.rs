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

/// `Time::now_nanos`: nanoseconds since the Unix epoch (wall clock).
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

/// `Time::monotonic_nanos`: nanoseconds on the monotonic clock.
pub fn monotonic_nanos() -> i64 {
    monotonic_start().elapsed().as_nanos() as i64
}
