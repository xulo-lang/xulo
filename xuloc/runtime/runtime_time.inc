// ── std::time ────────────────────────────────────────────────────────────
//
// Clocks, sleep, and the UTC calendar (std-spec modules/time.md). Timestamps
// are milliseconds (or nanoseconds for the `_nanos` readers) since the Unix
// epoch; the calendar uses pure integer arithmetic, so it does not depend on
// the host time zone or locale.

#define XR_TIME_MAX_MS 253402300799999ULL // 9999-12-31T23:59:59.999Z

static unsigned long long xr_time_u64(XVal v, const char *member) {
    if (v.tag != XT_INT) {
        xr_fail(member);
    }
    // A u64 above i64::MAX keeps its bits through the cast; the operations
    // below use unsigned arithmetic where the module is total.
    return (unsigned long long)v.i;
}

static long long xr_time_field(XVal v, long long fallback, const char *member) {
    if (v.tag == XT_NIL) {
        return fallback;
    }
    if (v.tag != XT_INT) {
        xr_fail(member);
    }
    return v.i;
}

static unsigned long long xr_time_wall_nanos(void) {
#ifdef _WIN32
    FILETIME file_time;
    GetSystemTimeAsFileTime(&file_time);
    ULARGE_INTEGER value;
    value.LowPart = file_time.dwLowDateTime;
    value.HighPart = file_time.dwHighDateTime;
    const unsigned long long epoch = 116444736000000000ULL; // 1601..1970 in 100ns
    if (value.QuadPart < epoch) {
        return 0;
    }
    return (value.QuadPart - epoch) * 100ULL;
#else
    struct timeval now;
    if (gettimeofday(&now, NULL) != 0 || now.tv_sec < 0) {
        return 0;
    }
    return (unsigned long long)now.tv_sec * 1000000000ULL +
           (unsigned long long)now.tv_usec * 1000ULL;
#endif
}

static unsigned long long xr_time_mono_nanos(void) {
#ifdef _WIN32
    static LARGE_INTEGER frequency;
    static int ready = 0;
    if (!ready) {
        QueryPerformanceFrequency(&frequency);
        ready = 1;
    }
    LARGE_INTEGER counter;
    QueryPerformanceCounter(&counter);
    if (frequency.QuadPart <= 0) {
        return (unsigned long long)GetTickCount64() * 1000000ULL;
    }
    return (unsigned long long)(counter.QuadPart / frequency.QuadPart) * 1000000000ULL +
           ((unsigned long long)(counter.QuadPart % frequency.QuadPart) * 1000000000ULL) /
               (unsigned long long)frequency.QuadPart;
#else
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) {
        return 0;
    }
    return (unsigned long long)now.tv_sec * 1000000000ULL +
           (unsigned long long)now.tv_nsec;
#endif
}

XVal xr_time_now(void) { return xr_int((long long)(xr_time_wall_nanos() / 1000000ULL)); }

XVal xr_time_now_nanos(void) { return xr_int((long long)xr_time_wall_nanos()); }

XVal xr_time_monotonic(void) {
    return xr_int((long long)(xr_time_mono_nanos() / 1000000ULL));
}

XVal xr_time_monotonic_nanos(void) { return xr_int((long long)xr_time_mono_nanos()); }

XVal xr_time_sleep(XVal milliseconds) {
    long long ms = (long long)xr_time_u64(milliseconds, "`std::time::sleep` expects a number");
    if (ms < 0) {
        // More than i64::MAX milliseconds: clamp to the host's maximum.
        ms = 253402300799999LL;
    }
#ifdef _WIN32
    while (ms > 0) {
        DWORD step = ms > 0x7fffffffLL ? 0x7fffffffUL : (DWORD)ms;
        Sleep(step);
        ms -= (long long)step;
    }
#else
    struct timespec request;
    request.tv_sec = (time_t)(ms / 1000);
    request.tv_nsec = (long)((ms % 1000) * 1000000);
    while (nanosleep(&request, &request) != 0 && errno == EINTR) {
    }
#endif
    return xr_nil();
}

static int xr_time_leap(long long year) {
    return (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
}

static int xr_time_month_days(long long year, long long month) {
    static const int days[12] = {31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31};
    if (month == 2 && xr_time_leap(year)) {
        return 29;
    }
    return days[month - 1];
}

// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
static long long xr_time_days_from_civil(long long year, long long month, long long day) {
    year -= month <= 2 ? 1 : 0;
    long long era = (year >= 0 ? year : year - 399) / 400;
    long long yoe = year - era * 400;
    long long doy = (153 * (month + (month > 2 ? -3 : 9)) + 2) / 5 + day - 1;
    long long doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    return era * 146097 + doe - 719468;
}

static void xr_time_civil_from_days(long long days, long long *year, long long *month,
                                    long long *day) {
    long long z = days + 719468;
    long long era = (z >= 0 ? z : z - 146096) / 146097;
    long long doe = z - era * 146097;
    long long yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    long long y = yoe + era * 400;
    long long doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    long long mp = (5 * doy + 2) / 153;
    long long d = doy - (153 * mp + 2) / 5 + 1;
    long long m = mp < 10 ? mp + 3 : mp - 9;
    *year = y + (m <= 2 ? 1 : 0);
    *month = m;
    *day = d;
}

XVal xr_time_format(XVal milliseconds) {
    unsigned long long value =
        xr_time_u64(milliseconds, "`std::time::format` expects a number");
    if (value > XR_TIME_MAX_MS) {
        xr_fail("std::time::format: timestamp is outside the calendar domain");
    }
    long long days = (long long)(value / 86400000ULL);
    long long rest = (long long)(value % 86400000ULL);
    long long year = 0;
    long long month = 0;
    long long day = 0;
    xr_time_civil_from_days(days, &year, &month, &day);
    long long hour = rest / 3600000LL;
    rest %= 3600000LL;
    long long minute = rest / 60000LL;
    rest %= 60000LL;
    long long second = rest / 1000LL;
    long long millisecond = rest % 1000LL;
    char buffer[40];
    snprintf(buffer, sizeof(buffer), "%04lld-%02lld-%02lldT%02lld:%02lld:%02lld.%03lldZ",
             year, month, day, hour, minute, second, millisecond);
    return xr_str(buffer);
}

static int xr_time_digits(const char *text, int at, int count, int *value) {
    int out = 0;
    for (int i = 0; i < count; i += 1) {
        char c = text[at + i];
        if (c < '0' || c > '9') {
            return 0;
        }
        out = out * 10 + (c - '0');
    }
    *value = out;
    return 1;
}

XVal xr_time_parse(XVal text) {
    if (text.tag != XT_STR) {
        return xr_err("std::time::parse expects a string");
    }
    XStr *s = xr_as_str(text);
    const char *data = s->data;
    int length = (int)s->bytes;
    int year = 0;
    int month = 0;
    int day = 0;
    int hour = 0;
    int minute = 0;
    int second = 0;
    int millisecond = 0;
    if (length < 20 || !xr_time_digits(data, 0, 4, &year) || data[4] != '-' ||
        !xr_time_digits(data, 5, 2, &month) || data[7] != '-' ||
        !xr_time_digits(data, 8, 2, &day) || (data[10] != 'T' && data[10] != ' ') ||
        !xr_time_digits(data, 11, 2, &hour) || data[13] != ':' ||
        !xr_time_digits(data, 14, 2, &minute) || data[16] != ':' ||
        !xr_time_digits(data, 17, 2, &second)) {
        return xr_err("invalid ISO 8601 timestamp");
    }
    int at = 19;
    if (data[at] == '.') {
        at += 1;
        int digits = 0;
        int fraction = 0;
        while (at < length && data[at] >= '0' && data[at] <= '9') {
            if (digits < 3) {
                fraction = fraction * 10 + (data[at] - '0');
            }
            digits += 1;
            at += 1;
        }
        if (digits < 1 || digits > 9) {
            return xr_err("invalid ISO 8601 timestamp");
        }
        while (digits < 3) {
            fraction *= 10;
            digits += 1;
        }
        millisecond = fraction;
    }
    if (at + 1 != length || data[at] != 'Z') {
        return xr_err("invalid ISO 8601 timestamp");
    }
    if (year < 1970 || year > 9999 || month < 1 || month > 12 || day < 1 ||
        day > xr_time_month_days(year, month) || hour < 0 || hour > 23 ||
        minute < 0 || minute > 59 || second < 0 || second > 59) {
        return xr_err("invalid ISO 8601 timestamp");
    }
    long long days = xr_time_days_from_civil(year, month, day);
    long long value = days * 86400000LL + hour * 3600000LL + minute * 60000LL +
                      second * 1000LL + millisecond;
    if (value < 0) {
        return xr_err("invalid ISO 8601 timestamp");
    }
    return xr_ok(xr_int(value));
}

XVal xr_time_parts(XVal milliseconds) {
    unsigned long long value =
        xr_time_u64(milliseconds, "`std::time::parts` expects a number");
    long long days = (long long)(value / 86400000ULL);
    long long rest = (long long)(value % 86400000ULL);
    long long year = 0;
    long long month = 0;
    long long day = 0;
    xr_time_civil_from_days(days, &year, &month, &day);
    const char *names[7] = {"year", "month", "day", "hour", "minute", "second",
                            "millisecond"};
    XVal values[7];
    values[0] = xr_int(year);
    values[1] = xr_int(month);
    values[2] = xr_int(day);
    values[3] = xr_int(rest / 3600000LL);
    values[4] = xr_int((rest % 3600000LL) / 60000LL);
    values[5] = xr_int((rest % 60000LL) / 1000LL);
    values[6] = xr_int(rest % 1000LL);
    return xr_struct("TimeParts", 7, names, values);
}

XVal xr_time_from_parts(XVal year_value, XVal month_value, XVal day_value,
                        XVal hour_value, XVal minute_value, XVal second_value,
                        XVal millisecond_value) {
    const char *member = "std::time::from_parts fields must be integers";
    long long year = xr_time_field(year_value, 0, member);
    long long month = xr_time_field(month_value, 0, member);
    long long day = xr_time_field(day_value, 0, member);
    long long hour = xr_time_field(hour_value, 0, member);
    long long minute = xr_time_field(minute_value, 0, member);
    long long second = xr_time_field(second_value, 0, member);
    long long millisecond = xr_time_field(millisecond_value, 0, member);
    if (year < 1970 || year > 9999 || month < 1 || month > 12 || day < 1 ||
        day > xr_time_month_days(year, month) || hour < 0 || hour > 23 ||
        minute < 0 || minute > 59 || second < 0 || second > 59 || millisecond < 0 ||
        millisecond > 999) {
        xr_fail("std::time::from_parts: fields are outside the calendar");
    }
    long long days = xr_time_days_from_civil(year, month, day);
    long long value = days * 86400000LL + hour * 3600000LL + minute * 60000LL +
                      second * 1000LL + millisecond;
    return xr_int(value);
}

XVal xr_time_weekday(XVal milliseconds) {
    unsigned long long value =
        xr_time_u64(milliseconds, "`std::time::weekday` expects a number");
    return xr_int((long long)(((value / 86400000ULL) + 4ULL) % 7ULL));
}
