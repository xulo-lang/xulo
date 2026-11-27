// ── std::math ────────────────────────────────────────────────────────────

static double xr_math_f(XVal value) {
    if (value.tag == XT_FLOAT) {
        return value.f;
    }
    if (value.tag == XT_INT) {
        return (double)value.i;
    }
    xr_fail("`std::math` expects a number");
    return 0.0;
}

static long long xr_math_i(XVal value) {
    if (value.tag == XT_INT) {
        return value.i;
    }
    if (value.tag == XT_FLOAT) {
        return (long long)value.f;
    }
    xr_fail("`std::math` expects a number");
    return 0;
}

XVal xr_math_abs(XVal x) {
    if (x.tag == XT_INT) {
        return xr_int(x.i < 0 ? -x.i : x.i);
    }
    return xr_float(fabs(xr_math_f(x)));
}

XVal xr_math_min(XVal a, XVal b) {
    if (a.tag == XT_INT && b.tag == XT_INT) {
        return b.i < a.i ? b : a;
    }
    return xr_math_f(b) < xr_math_f(a) ? b : a;
}

XVal xr_math_max(XVal a, XVal b) {
    if (a.tag == XT_INT && b.tag == XT_INT) {
        return b.i > a.i ? b : a;
    }
    return xr_math_f(b) > xr_math_f(a) ? b : a;
}

XVal xr_math_clamp(XVal x, XVal lo, XVal hi) {
    if (x.tag == XT_INT && lo.tag == XT_INT && hi.tag == XT_INT) {
        if (x.i < lo.i) {
            return lo;
        }
        if (x.i > hi.i) {
            return hi;
        }
        return x;
    }
    if (xr_math_f(x) < xr_math_f(lo)) {
        return lo;
    }
    if (xr_math_f(x) > xr_math_f(hi)) {
        return hi;
    }
    return x;
}

XVal xr_math_sign(XVal x) {
    double value = xr_math_f(x);
    if (value > 0.0) {
        return x.tag == XT_INT ? xr_int(1) : xr_float(1.0);
    }
    if (value < 0.0) {
        return x.tag == XT_INT ? xr_int(-1) : xr_float(-1.0);
    }
    return x.tag == XT_INT ? xr_int(0) : xr_float(0.0);
}

#define XR_MATH_FN(name, expr)                                                                     \
    XVal xr_math_##name(XVal x) { return xr_float(expr(xr_math_f(x))); }

XR_MATH_FN(sqrt, sqrt)
XR_MATH_FN(cbrt, cbrt)
XR_MATH_FN(sin, sin)
XR_MATH_FN(cos, cos)
XR_MATH_FN(tan, tan)
XR_MATH_FN(asin, asin)
XR_MATH_FN(acos, acos)
XR_MATH_FN(atan, atan)
XR_MATH_FN(log, log)
XR_MATH_FN(log2, log2)
XR_MATH_FN(log10, log10)
XR_MATH_FN(log1p, log1p)
XR_MATH_FN(exp, exp)
XR_MATH_FN(expm1, expm1)

XVal xr_math_atan2(XVal y, XVal x) {
    return xr_float(atan2(xr_math_f(y), xr_math_f(x)));
}

XVal xr_math_pow(XVal base, XVal exponent) {
    return xr_float(pow(xr_math_f(base), xr_math_f(exponent)));
}

XVal xr_math_hypot(XVal a, XVal b) {
    return xr_float(hypot(xr_math_f(a), xr_math_f(b)));
}

XVal xr_math_floor(XVal x) { return xr_int((long long)floor(xr_math_f(x))); }
XVal xr_math_ceil(XVal x) { return xr_int((long long)ceil(xr_math_f(x))); }
XVal xr_math_round(XVal x) { return xr_int((long long)round(xr_math_f(x))); }
XVal xr_math_trunc(XVal x) { return xr_int((long long)trunc(xr_math_f(x))); }

static long long xr_math_pop64(uint64_t bits) {
    long long count = 0;
    while (bits != 0) {
        count += (long long)(bits & 1);
        bits >>= 1;
    }
    return count;
}

XVal xr_math_pop_count(XVal x) { return xr_int(xr_math_pop64((uint64_t)xr_math_i(x))); }

XVal xr_math_count_leading_zeros(XVal x) {
    uint64_t bits = (uint64_t)xr_math_i(x);
    if (bits == 0) {
        return xr_int(64);
    }
    long long count = 0;
    while ((bits & 0x8000000000000000ULL) == 0) {
        bits <<= 1;
        count += 1;
    }
    return xr_int(count);
}

XVal xr_math_count_trailing_zeros(XVal x) {
    uint64_t bits = (uint64_t)xr_math_i(x);
    if (bits == 0) {
        return xr_int(64);
    }
    long long count = 0;
    while ((bits & 1) == 0) {
        bits >>= 1;
        count += 1;
    }
    return xr_int(count);
}

XVal xr_math_rotate_left(XVal x, XVal amount) {
    long long shift = xr_math_i(amount) & 63;
    if (shift == 0) {
        return x;
    }
    uint64_t bits = (uint64_t)xr_math_i(x);
    return xr_int((long long)((bits << shift) | (bits >> (64 - shift))));
}

XVal xr_math_rotate_right(XVal x, XVal amount) {
    long long shift = xr_math_i(amount) & 63;
    if (shift == 0) {
        return x;
    }
    uint64_t bits = (uint64_t)xr_math_i(x);
    return xr_int((long long)((bits >> shift) | (bits << (64 - shift))));
}

XVal xr_math_wrapping_add(XVal a, XVal b) {
    return xr_int((long long)((uint64_t)xr_math_i(a) + (uint64_t)xr_math_i(b)));
}

XVal xr_math_wrapping_sub(XVal a, XVal b) {
    return xr_int((long long)((uint64_t)xr_math_i(a) - (uint64_t)xr_math_i(b)));
}

XVal xr_math_wrapping_mul(XVal a, XVal b) {
    return xr_int((long long)((uint64_t)xr_math_i(a) * (uint64_t)xr_math_i(b)));
}

XVal xr_math_to_fixed(XVal x, XVal digits) {
    long long d = xr_math_i(digits);
    if (d < 0 || d > 15) {
        // `xr_fail` does not return; the assignment keeps the compiler happy.
        xr_fail("`std::math::to_fixed` digits must be an integer in 0..=15");
        return xr_str("");
    }
    double value = xr_math_f(x);
    if (isnan(value) || isinf(value)) {
        return xr_format_float(value);
    }
    double factor = pow(10.0, (double)d);
    double scaled = value * factor;
    char buffer[128];
    if (isinf(scaled)) {
        snprintf(buffer, sizeof(buffer), "%.*f", (int)d, value);
    } else {
        snprintf(buffer, sizeof(buffer), "%.*f", (int)d, round(scaled) / factor);
    }
    return xr_str(buffer);
}

static uint64_t xr_math_rng_state = 0;

static uint64_t xr_math_rand64(void) {
    if (xr_math_rng_state == 0) {
        uint64_t seed = (uint64_t)time(NULL);
        seed ^= (uint64_t)(uintptr_t)&xr_math_rng_state;
        seed ^= (uint64_t)(unsigned long long)clock() << 17;
        seed ^= (uint64_t)(unsigned long long)XR_GETPID() << 33;
        xr_math_rng_state = seed == 0 ? 0x9E3779B97F4A7C15ULL : seed;
    }
    // xorshift64*
    uint64_t x = xr_math_rng_state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    xr_math_rng_state = x;
    return x * 0x2545F4914F6CDD1DULL;
}

XVal xr_math_random(void) {
    return xr_float((double)(xr_math_rand64() >> 11) * (1.0 / 9007199254740992.0));
}

XVal xr_math_random_int(XVal lo, XVal hi) {
    long long low = xr_math_i(lo);
    long long high = xr_math_i(hi);
    if (low > high) {
        xr_fail("`std::math::random_int` needs `lo <= hi`");
        return xr_int(low);
    }
    uint64_t span = (uint64_t)high - (uint64_t)low;
    uint64_t offset = span == UINT64_MAX ? xr_math_rand64() : xr_math_rand64() % (span + 1);
    return xr_int((long long)((uint64_t)low + offset));
}

XVal xr_os_getenv(XVal name) {
    if (name.tag != XT_STR) {
        return xr_ok(xr_nil());
    }
    const char *value = getenv(xr_as_str(name)->data);
    if (value == NULL) {
        return xr_ok(xr_nil());
    }
    return xr_ok(xr_str(value));
}

XVal xr_os_platform(void) {
#ifdef _WIN32
    return xr_str("windows");
#elif defined(__APPLE__)
    return xr_str("macos");
#else
    return xr_str("linux");
#endif
}

XVal xr_os_cwd(void) {
    char buffer[4096];
#ifdef _WIN32
    if (_getcwd(buffer, (int)sizeof(buffer)) == NULL) {
        return xr_err("cannot read the current directory");
    }
#else
    if (getcwd(buffer, sizeof(buffer)) == NULL) {
        return xr_err("cannot read the current directory");
    }
#endif
    return xr_ok(xr_str(buffer));
}

XVal xr_os_chdir(XVal path) {
    if (path.tag != XT_STR) {
        return xr_err("std::os::chdir expects a string path");
    }
#ifdef _WIN32
    if (_chdir(xr_as_str(path)->data) != 0) {
        return xr_err("cannot change the current directory");
    }
#else
    if (chdir(xr_as_str(path)->data) != 0) {
        return xr_err("cannot change the current directory");
    }
#endif
    return xr_ok(xr_nil());
}

XVal xr_os_tmp_dir(void) {
#ifdef _WIN32
    char buffer[MAX_PATH + 1];
    DWORD size = GetTempPathA(MAX_PATH + 1, buffer);
    if (size == 0) {
        return xr_str(".");
    }
    long long n = (long long)size;
    while (n > 1 && buffer[n - 1] == '\\') {
        n -= 1;
    }
    return xr_str_n(buffer, n);
#else
    const char *tmp = getenv("TMPDIR");
    if (tmp == NULL || tmp[0] == '\0') {
        tmp = "/tmp";
    }
    long long n = (long long)strlen(tmp);
    while (n > 1 && tmp[n - 1] == '/') {
        n -= 1;
    }
    return xr_str_n(tmp, n);
#endif
}

XVal xr_os_home_dir(void) {
    const char *home = getenv("HOME");
#ifdef _WIN32
    if (home == NULL || home[0] == '\0') {
        home = getenv("USERPROFILE");
    }
#endif
    if (home == NULL || home[0] == '\0') {
        return xr_err("cannot read the home directory");
    }
    return xr_ok(xr_str(home));
}

XVal xr_os_setenv(XVal name, XVal value) {
    if (name.tag != XT_STR || value.tag != XT_STR) {
        return xr_err("std::os::setenv expects two strings");
    }
#ifdef _WIN32
    if (_putenv_s(xr_as_str(name)->data, xr_as_str(value)->data) != 0) {
        return xr_err("cannot set the environment variable");
    }
#else
    if (setenv(xr_as_str(name)->data, xr_as_str(value)->data, 1) != 0) {
        return xr_err("cannot set the environment variable");
    }
#endif
    return xr_ok(xr_nil());
}

XVal xr_os_unsetenv(XVal name) {
    if (name.tag != XT_STR) {
        return xr_err("std::os::unsetenv expects a string name");
    }
#ifdef _WIN32
    if (_putenv_s(xr_as_str(name)->data, "") != 0) {
        return xr_err("cannot unset the environment variable");
    }
#else
    if (unsetenv(xr_as_str(name)->data) != 0) {
        return xr_err("cannot unset the environment variable");
    }
#endif
    return xr_ok(xr_nil());
}

XVal xr_os_exec_path(void) {
#ifdef _WIN32
    char buffer[MAX_PATH + 1];
    DWORD size = GetModuleFileNameA(NULL, buffer, MAX_PATH + 1);
    if (size == 0) {
        return xr_err("cannot read the executable path");
    }
    return xr_ok(xr_str_n(buffer, (long long)size));
#elif defined(__APPLE__)
    uint32_t size = 0;
    _NSGetExecutablePath(NULL, &size);
    char *buffer = (char *)xr_alloc((size_t)size + 1);
    if (_NSGetExecutablePath(buffer, &size) != 0) {
        return xr_err("cannot read the executable path");
    }
    buffer[size] = '\0';
    return xr_ok(xr_str(buffer));
#else
    char buffer[4096];
    ssize_t size = readlink("/proc/self/exe", buffer, sizeof(buffer) - 1);
    if (size <= 0) {
        return xr_err("cannot read the executable path");
    }
    buffer[size] = '\0';
    return xr_ok(xr_str(buffer));
#endif
}

XVal xr_os_host_name(void) {
#ifdef _WIN32
    char buffer[256];
    DWORD size = (DWORD)sizeof(buffer);
    if (!GetComputerNameA(buffer, &size)) {
        return xr_err("cannot read the host name");
    }
    return xr_ok(xr_str_n(buffer, (long long)size));
#else
    char buffer[256];
    if (gethostname(buffer, sizeof(buffer)) != 0) {
        return xr_err("cannot read the host name");
    }
    buffer[sizeof(buffer) - 1] = '\0';
    return xr_ok(xr_str(buffer));
#endif
}

XVal xr_os_user_name(void) {
#ifdef _WIN32
    char buffer[256];
    DWORD size = (DWORD)sizeof(buffer);
    if (!GetUserNameA(buffer, &size)) {
        return xr_err("cannot read the user name");
    }
    long long n = (long long)size;
    if (n > 0 && buffer[n - 1] == '\0') {
        n -= 1;
    }
    return xr_ok(xr_str_n(buffer, n));
#else
    struct passwd *entry = getpwuid(getuid());
    if (entry == NULL || entry->pw_name == NULL) {
        return xr_err("cannot read the user name");
    }
    return xr_ok(xr_str(entry->pw_name));
#endif
}

XVal xr_os_pid(void) {
#ifdef _WIN32
    return xr_int((long long)GetCurrentProcessId());
#else
    return xr_int((long long)getpid());
#endif
}

XVal xr_os_arch(void) {
#if defined(__x86_64__) || defined(_M_X64)
    return xr_str("x86_64");
#elif defined(__aarch64__) || defined(_M_ARM64)
    return xr_str("aarch64");
#elif defined(__i386__) || defined(_M_IX86)
    return xr_str("x86");
#else
    return xr_str("unknown");
#endif
}

XVal xr_os_cpu_count(void) {
#ifdef _WIN32
    SYSTEM_INFO info;
    GetSystemInfo(&info);
    return xr_int((long long)info.dwNumberOfProcessors);
#else
    long count = sysconf(_SC_NPROCESSORS_ONLN);
    if (count < 1) {
        count = 1;
    }
    return xr_int((long long)count);
#endif
}

static XVal xr_process_argv;

void xr_set_argv(int argc, char **argv) {
    xr_process_argv = xr_list_new();
    for (int i = 0; i < argc; i += 1) {
        xr_list_push(xr_process_argv, xr_str(argv[i]));
    }
}

XVal xr_process_args(void) { return xr_process_argv; }

XVal xr_process_exit(XVal code) {
    fflush(stdout);
    fflush(stderr);
    exit((int)code.i);
    return xr_nil();
}

