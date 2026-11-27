// ── std::io readers and terminal probes ──────────────────────────────────
//
// The writes live with the renderer (`xr_io_print` / `xr_io_eprint` /
// `xr_io_flush` in runtime_render.inc); this module adds the rest of
// std-spec modules/io.md: the two tty probes and the three stdin readers.
// Every reader decodes strict UTF-8 and reports the first invalid byte's
// offset, and `read_line` / `read_n` distinguish end of input (`Ok(null)`)
// from an empty line (`Ok("")`).

#define XR_IO_MAX_READ_N 16777216LL

// Bytes from a previous `read_n` held back because they were an incomplete
// scalar at the cut (std-spec io.md). The bootstrap runtime is single-threaded.
static char *xr_io_pending = NULL;
static long long xr_io_pending_len = 0;

// stdin is a byte stream: disable the Windows text-mode CRLF / Ctrl-Z
// translation so `read_n` counts the same bytes stage 0 sees.
static void xr_io_binary_stdin(void) {
#ifdef _WIN32
    static int done = 0;
    if (!done) {
        _setmode(_fileno(stdin), _O_BINARY);
        done = 1;
    }
#endif
}

// Stage 0's stdout/stderr are byte streams; Windows text mode would turn every
// `\n` into `\r\n`. Match the Rust runtime so captured child output and the
// program's own lines are byte-identical.
static void xr_io_binary_output(void) {
#ifdef _WIN32
    static int done = 0;
    if (!done) {
        _setmode(_fileno(stdout), _O_BINARY);
        _setmode(_fileno(stderr), _O_BINARY);
        done = 1;
    }
#endif
}

// The number of leading valid bytes. `incomplete` is set when the first
// problem is a scalar cut off by the end of the buffer (Rust's
// `Utf8Error::error_len() == None`) rather than an invalid byte.
static long long xr_io_utf8_valid_up_to(const unsigned char *s, long long n, int *incomplete) {
    long long at = 0;
    *incomplete = 0;
    while (at < n) {
        unsigned char c = s[at];
        long long len = 0;
        if (c < 0x80) {
            at += 1;
            continue;
        }
        if (c >= 0xC2 && c <= 0xDF) {
            len = 2;
        } else if (c >= 0xE0 && c <= 0xEF) {
            len = 3;
        } else if (c >= 0xF0 && c <= 0xF4) {
            len = 4;
        } else {
            return at;
        }
        long long available = n - at;
        if (available < len) {
            for (long long k = 1; k < available; k += 1) {
                if ((s[at + k] & 0xC0) != 0x80) {
                    return at;
                }
            }
            *incomplete = 1;
            return at;
        }
        for (long long k = 1; k < len; k += 1) {
            if ((s[at + k] & 0xC0) != 0x80) {
                return at;
            }
        }
        // Reject overlong forms, UTF-16 surrogates, and scalars above U+10FFFF.
        if (len == 3) {
            if (c == 0xE0 && s[at + 1] < 0xA0) {
                return at;
            }
            if (c == 0xED && s[at + 1] >= 0xA0) {
                return at;
            }
        } else if (len == 4) {
            if (c == 0xF0 && s[at + 1] < 0x90) {
                return at;
            }
            if (c == 0xF4 && s[at + 1] >= 0x90) {
                return at;
            }
        }
        at += len;
    }
    return at;
}

static XVal xr_io_invalid_utf8(long long offset) {
    char message[96];
    snprintf(message, sizeof(message), "cannot read from stdin: invalid UTF-8 at byte %lld", offset);
    return xr_err(message);
}

#ifdef _WIN32
// `_isatty` is true for character devices such as NUL, while stage 0's
// `IsTerminal` asks whether the handle is a console. Match stage 0 so a
// redirected or null stream reads as `false`.
static int xr_io_is_console(DWORD which) {
    DWORD mode = 0;
    HANDLE handle = GetStdHandle(which);
    if (handle == NULL || handle == INVALID_HANDLE_VALUE) {
        return 0;
    }
    return GetConsoleMode(handle, &mode) != 0;
}
#endif

XVal xr_io_stdin_is_tty(void) {
#ifdef _WIN32
    return xr_bool(xr_io_is_console(STD_INPUT_HANDLE));
#else
    return xr_bool(isatty(fileno(stdin)));
#endif
}

XVal xr_io_stdout_is_tty(void) {
#ifdef _WIN32
    return xr_bool(xr_io_is_console(STD_OUTPUT_HANDLE));
#else
    return xr_bool(isatty(fileno(stdout)));
#endif
}

XVal xr_io_read_line(void) {
    xr_io_binary_stdin();
    long long cap = 128;
    long long len = 0;
    char *data = (char *)xr_alloc((size_t)cap);
    int ch = 0;
    while ((ch = fgetc(stdin)) != EOF) {
        if (len + 1 >= cap) {
            long long next_cap = cap * 2;
            char *next = (char *)xr_alloc((size_t)next_cap);
            memcpy(next, data, (size_t)len);
            data = next;
            cap = next_cap;
        }
        data[len] = (char)ch;
        len += 1;
        if (ch == '\n') {
            break;
        }
    }
    if (ferror(stdin)) {
        return xr_err("cannot read from stdin");
    }
    if (ch == EOF && len == 0) {
        return xr_ok(xr_nil());
    }
    if (len > 0 && data[len - 1] == '\n') {
        len -= 1;
        if (len > 0 && data[len - 1] == '\r') {
            len -= 1;
        }
    }
    int incomplete = 0;
    long long valid = xr_io_utf8_valid_up_to((const unsigned char *)data, len, &incomplete);
    if (valid != len) {
        return xr_io_invalid_utf8(valid);
    }
    return xr_ok(xr_str_n(data, len));
}

XVal xr_io_read(void) {
    xr_io_binary_stdin();
    long long bytes = 0;
    char *data = xr_read_stream(stdin, &bytes);
    if (ferror(stdin)) {
        return xr_err("cannot read from stdin");
    }
    int incomplete = 0;
    long long valid = xr_io_utf8_valid_up_to((const unsigned char *)data, bytes, &incomplete);
    if (valid != bytes) {
        return xr_io_invalid_utf8(valid);
    }
    return xr_ok(xr_str_n(data, bytes));
}

XVal xr_io_read_n(XVal max_value) {
    long long max = 0;
    if (max_value.tag == XT_INT) {
        max = max_value.i;
    } else if (max_value.tag == XT_FLOAT) {
        double amount = max_value.f;
        if (!isfinite(amount)) {
            return xr_err("std::io::read_n: max must be a number");
        }
        max = (long long)amount; // fractional values truncate toward zero
    } else {
        return xr_err("std::io::read_n: max must be a number");
    }
    if (max < 1 || max > XR_IO_MAX_READ_N) {
        char message[96];
        snprintf(message, sizeof(message),
                 "std::io::read_n: max must be in 1..=16777216, found %lld", max);
        return xr_err(message);
    }

    xr_io_binary_stdin();
    // A previous call may have held back an incomplete scalar; the next call
    // continues with those bytes (std-spec io.md).
    long long len = xr_io_pending_len;
    char *data = xr_io_pending;
    xr_io_pending = NULL;
    xr_io_pending_len = 0;
    long long cap = len + 64;
    if (len < max && cap > max) {
        cap = max;
    }
    if (cap < len + 1) {
        cap = len + 1;
    }
    if (data == NULL) {
        data = (char *)xr_alloc((size_t)cap);
    } else if (cap > len) {
        char *next = (char *)xr_alloc((size_t)cap);
        memcpy(next, data, (size_t)len);
        data = next;
    }

    while (len < max) {
        if (len == cap) {
            long long next_cap = cap * 2;
            if (next_cap > max) {
                next_cap = max;
            }
            if (next_cap <= cap) {
                next_cap = cap + 1;
            }
            char *next = (char *)xr_alloc((size_t)next_cap);
            memcpy(next, data, (size_t)len);
            data = next;
            cap = next_cap;
        }
        int ch = fgetc(stdin);
        if (ch == EOF) {
            break;
        }
        data[len] = (char)ch;
        len += 1;
    }
    if (ferror(stdin)) {
        return xr_err("cannot read from stdin");
    }
    if (len == 0) {
        return xr_ok(xr_nil());
    }

    int incomplete = 0;
    long long valid = xr_io_utf8_valid_up_to((const unsigned char *)data, len, &incomplete);
    if (valid == len) {
        return xr_ok(xr_str_n(data, len));
    }
    if (incomplete) {
        if (valid == 0) {
            // One scalar is longer than `max`: hold it for a larger read
            // instead of splitting it.
            xr_io_pending = data;
            xr_io_pending_len = len;
            return xr_err("cannot read from stdin: incomplete UTF-8 sequence at the read limit");
        }
        long long tail = len - valid;
        char *held = (char *)xr_alloc((size_t)tail);
        memcpy(held, data + valid, (size_t)tail);
        xr_io_pending = held;
        xr_io_pending_len = tail;
        return xr_ok(xr_str_n(data, valid));
    }
    return xr_io_invalid_utf8(valid);
}
