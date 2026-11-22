// ── formatting ───────────────────────────────────────────────────────────

// Rust's `Display` for f64 does not switch to exponent notation for ordinary
// magnitudes; `%g` does (`1000.0` becomes `1e+03`). Expand the shortest
// round-tripping form back to plain decimal notation.
static void xr_expand_exponent(const char *text, char *out, size_t cap) {
    const char *mark = strchr(text, 'e');
    if (mark == NULL) {
        snprintf(out, cap, "%s", text);
        return;
    }
    char mantissa[64];
    size_t mlen = (size_t)(mark - text);
    if (mlen >= sizeof(mantissa)) {
        mlen = sizeof(mantissa) - 1;
    }
    memcpy(mantissa, text, mlen);
    mantissa[mlen] = '\0';
    int exponent = atoi(mark + 1);
    int negative = 0;
    char *digits_text = mantissa;
    if (*digits_text == '-' || *digits_text == '+') {
        negative = *digits_text == '-';
        digits_text += 1;
    }
    char digits[64];
    int count = 0;
    int dot = -1;
    for (char *p = digits_text; *p != '\0'; p += 1) {
        if (*p == '.') {
            dot = count;
        } else if (count < (int)sizeof(digits) - 1) {
            digits[count] = *p;
            count += 1;
        }
    }
    digits[count] = '\0';
    int point = (dot >= 0 ? dot : count) + exponent;
    size_t pos = 0;
    if (negative && pos + 1 < cap) {
        out[pos++] = '-';
    }
    if (point <= 0) {
        if (pos + 2 < cap) {
            out[pos++] = '0';
            out[pos++] = '.';
        }
        for (int i = 0; i < -point && pos + 1 < cap; i += 1) {
            out[pos++] = '0';
        }
        for (int i = 0; i < count && pos + 1 < cap; i += 1) {
            out[pos++] = digits[i];
        }
    } else if (point >= count) {
        for (int i = 0; i < count && pos + 1 < cap; i += 1) {
            out[pos++] = digits[i];
        }
        for (int i = count; i < point && pos + 1 < cap; i += 1) {
            out[pos++] = '0';
        }
    } else {
        for (int i = 0; i < point && pos + 1 < cap; i += 1) {
            out[pos++] = digits[i];
        }
        if (pos + 1 < cap) {
            out[pos++] = '.';
        }
        for (int i = point; i < count && pos + 1 < cap; i += 1) {
            out[pos++] = digits[i];
        }
    }
    out[pos] = '\0';
}

static XVal xr_format_float(double f) {
    if (isnan(f)) {
        return xr_str("NaN");
    }
    if (isinf(f)) {
        return xr_str(f > 0 ? "Infinity" : "-Infinity");
    }
    if (f == 0.0) {
        return xr_str("0");
    }
    // Shortest decimal that round-trips, like Rust's `Display` for f64. The
    // old `%.15g`/`%.17g` pair printed extra digits for values such as PI
    // (drift D-02).
    char buffer[64];
    for (int precision = 1; precision <= 17; precision += 1) {
        snprintf(buffer, sizeof(buffer), "%.*g", precision, f);
        if (strtod(buffer, NULL) == f) {
            char plain[512];
            xr_expand_exponent(buffer, plain, sizeof(plain));
            return xr_str(plain);
        }
    }
    snprintf(buffer, sizeof(buffer), "%.17g", f);
    char plain[512];
    xr_expand_exponent(buffer, plain, sizeof(plain));
    return xr_str(plain);
}

static void xr_append_str(char **buffer, long long *len, long long *cap, const char *data,
                           long long bytes) {
    if (*len + bytes + 1 > *cap) {
        long long next_cap = *cap == 0 ? 64 : *cap;
        while (next_cap < *len + bytes + 1) {
            next_cap *= 2;
        }
        char *next = (char *)xr_alloc((size_t)next_cap);
        if (*len > 0) {
            memcpy(next, *buffer, (size_t)*len);
        }
        *buffer = next;
        *cap = next_cap;
    }
    memcpy(*buffer + *len, data, (size_t)bytes);
    *len += bytes;
    (*buffer)[*len] = '\0';
}

static void xr_render_value(XVal v, char **buffer, long long *len, long long *cap) {
    switch (v.tag) {
    case XT_NIL:
        xr_append_str(buffer, len, cap, "null", 4);
        return;
    case XT_INT: {
        char tmp[32];
        int n = snprintf(tmp, sizeof(tmp), "%lld", v.i);
        xr_append_str(buffer, len, cap, tmp, n);
        return;
    }
    case XT_FLOAT: {
        XVal s = xr_format_float(v.f);
        XStr *str = xr_as_str(s);
        xr_append_str(buffer, len, cap, str->data, str->bytes);
        return;
    }
    case XT_BOOL:
        if (v.i != 0) {
            xr_append_str(buffer, len, cap, "true", 4);
        } else {
            xr_append_str(buffer, len, cap, "false", 5);
        }
        return;
    case XT_STR: {
        XStr *s = xr_as_str(v);
        xr_append_str(buffer, len, cap, s->data, s->bytes);
        return;
    }
    case XT_LIST: {
        XList *l = xr_as_list(v);
        xr_append_str(buffer, len, cap, "[", 1);
        for (long long i = 0; i < l->len; i += 1) {
            if (i > 0) {
                xr_append_str(buffer, len, cap, ", ", 2);
            }
            xr_render_value(l->items[i], buffer, len, cap);
        }
        xr_append_str(buffer, len, cap, "]", 1);
        return;
    }
    case XT_SB: {
        XSb *s = (XSb *)v.p;
        xr_append_str(buffer, len, cap, s->data == NULL ? "" : s->data, s->bytes);
        return;
    }
    case XT_SET: {
        XSet *s = (XSet *)v.p;
        xr_append_str(buffer, len, cap, "Set([", 5);
        for (long long i = 0; i < s->len; i += 1) {
            if (i > 0) {
                xr_append_str(buffer, len, cap, ", ", 2);
            }
            xr_render_value(s->items[i], buffer, len, cap);
        }
        xr_append_str(buffer, len, cap, "])", 2);
        return;
    }
    case XT_OBJ: {
        XObj *o = xr_as_obj(v);
        // A brace map literal renders like stage 0's object value (`{ k: v }`);
        // every other field table keeps its type prefix.
        if (o->variant != NULL || o->debug || strcmp(o->type, "Map") != 0) {
            xr_append_str(buffer, len, cap, o->type, (long long)strlen(o->type));
        }
        if (o->variant != NULL) {
            xr_append_str(buffer, len, cap, ".", 1);
            xr_append_str(buffer, len, cap, o->variant, (long long)strlen(o->variant));
            if (o->payload_len == 1) {
                xr_append_str(buffer, len, cap, "(", 1);
                xr_render_value(o->payload[0], buffer, len, cap);
                xr_append_str(buffer, len, cap, ")", 1);
            } else if (o->payload_len > 1) {
                xr_append_str(buffer, len, cap, "(", 1);
                for (long long i = 0; i < o->payload_len; i += 1) {
                    if (i > 0) {
                        xr_append_str(buffer, len, cap, ", ", 2);
                    }
                    xr_render_value(o->payload[i], buffer, len, cap);
                }
                xr_append_str(buffer, len, cap, ")", 1);
            }
            return;
        }
        if (o->debug) {
            // `#[derive(Debug)]` struct: `Type(field: value, …)`.
            xr_append_str(buffer, len, cap, "(", 1);
            for (long long i = 0; i < o->nfields; i += 1) {
                if (i > 0) {
                    xr_append_str(buffer, len, cap, ", ", 2);
                }
                const char *name = o->fields[i].name;
                xr_append_str(buffer, len, cap, name, (long long)strlen(name));
                xr_append_str(buffer, len, cap, ": ", 2);
                xr_render_value(o->fields[i].value, buffer, len, cap);
            }
            xr_append_str(buffer, len, cap, ")", 1);
            return;
        }
        xr_append_str(buffer, len, cap, "{ ", 2);
        for (long long i = 0; i < o->nfields; i += 1) {
            if (i > 0) {
                xr_append_str(buffer, len, cap, ", ", 2);
            }
            const char *name = o->fields[i].name;
            xr_append_str(buffer, len, cap, name, (long long)strlen(name));
            xr_append_str(buffer, len, cap, ": ", 2);
            xr_render_value(o->fields[i].value, buffer, len, cap);
        }
        xr_append_str(buffer, len, cap, " }", 2);
        return;
    }
    default:
        xr_append_str(buffer, len, cap, "?", 1);
    }
}

XVal xr_str_of(XVal v) {
    char *buffer = NULL;
    long long len = 0;
    long long cap = 0;
    xr_render_value(v, &buffer, &len, &cap);
    if (buffer == NULL) {
        return xr_str("");
    }
    return xr_str_n(buffer, len);
}

// `std::io::println` returns `Result<unit, Error>`; the prelude `print` uses
// `xr_print_args` and returns unit.
XVal xr_print(XVal v) {
    XVal s = xr_str_of(v);
    XStr *str = xr_as_str(s);
    fwrite(str->data, 1, (size_t)str->bytes, stdout);
    fputc('\n', stdout);
    fflush(stdout);
    return xr_ok(xr_nil());
}

XVal xr_eprintln(XVal v) {
    XVal s = xr_str_of(v);
    XStr *str = xr_as_str(s);
    fflush(stdout);
    fwrite(str->data, 1, (size_t)str->bytes, stderr);
    fputc('\n', stderr);
    fflush(stderr);
    return xr_ok(xr_nil());
}

// The prelude `print` / `println` / `eprint` / `eprintln` are variadic: each
// value renders with `xr_str_of`, consecutive values are separated by one
// space, and the line terminator is written once.
static void xr_write_values(FILE *stream, long long n, XVal *values) {
    for (long long i = 0; i < n; i += 1) {
        if (i > 0) {
            fputc(' ', stream);
        }
        XVal s = xr_str_of(values[i]);
        XStr *str = xr_as_str(s);
        fwrite(str->data, 1, (size_t)str->bytes, stream);
    }
}

XVal xr_print_args(long long n, XVal *args) {
    xr_write_values(stdout, n, args);
    fputc('\n', stdout);
    fflush(stdout);
    return xr_nil();
}

XVal xr_eprint_args(long long n, XVal *args) {
    fflush(stdout);
    xr_write_values(stderr, n, args);
    fputc('\n', stderr);
    fflush(stderr);
    return xr_nil();
}

// `std::io::print` and friends take exactly one string and write it with no
// rendering or spacing of their own (std-spec `modules/io.md`).
XVal xr_io_print(XVal v) {
    XVal s = xr_str_of(v);
    XStr *str = xr_as_str(s);
    fwrite(str->data, 1, (size_t)str->bytes, stdout);
    fflush(stdout);
    return xr_ok(xr_nil());
}

XVal xr_io_eprint(XVal v) {
    XVal s = xr_str_of(v);
    XStr *str = xr_as_str(s);
    fflush(stdout);
    fwrite(str->data, 1, (size_t)str->bytes, stderr);
    fflush(stderr);
    return xr_ok(xr_nil());
}

XVal xr_io_flush(void) {
    fflush(stdout);
    return xr_ok(xr_nil());
}

