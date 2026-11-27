// ── std::fmt ─────────────────────────────────────────────────────────────
//
// `std::fmt::format(template, args)`: literal text plus `%` directives, with
// the verbs and layout vocabulary of `std-spec/modules/fmt.md`. The engine is
// a port of `crates/xulo-std/src/fmt.rs`: parse the template, check the
// directive count against the argument list, then render each directive.
// Every malformed template and mismatched operand is a runtime failure.

#define XR_FMT_MAX_WIDTH (1LL << 20)

typedef struct {
    int literal;
    const char *text;
    long long bytes;
    char verb;
    int dash;
    int plus;
    int zero;
    int space;
    int alt;
    long long width;     // -1 when omitted
    long long precision; // -1 when omitted
} XrFmtPiece;

static int xr_fmt_is_numeric_verb(char verb) {
    return verb == 'd' || verb == 'x' || verb == 'X' || verb == 'o' || verb == 'b' ||
           verb == 'f' || verb == 'e' || verb == 'E' || verb == 'g' || verb == 'G';
}

static int xr_fmt_is_alt_verb(char verb) {
    return verb == 'x' || verb == 'X' || verb == 'o' || verb == 'b' || verb == 'v' ||
           verb == 's';
}

static int xr_fmt_is_verb(char verb) {
    return verb == 'v' || verb == 's' || verb == 'q' || verb == 't' || verb == 'c' ||
           xr_fmt_is_numeric_verb(verb);
}

static void xr_fmt_fail(const char *message) {
    char buffer[256];
    snprintf(buffer, sizeof(buffer), "`std::fmt::format` %s", message);
    xr_fail(buffer);
}

// Consume a run of digits. Returns -1 when the first byte is not a digit and
// -2 after reporting an overwide value. `what` names the field in the message.
static long long xr_fmt_number(const char *text, long long *at, long long bytes,
                               const char *what) {
    long long start = *at;
    long long value = 0;
    int too_large = 0;
    while (*at < bytes && text[*at] >= '0' && text[*at] <= '9') {
        int digit = text[*at] - '0';
        if (value > XR_FMT_MAX_WIDTH) {
            too_large = 1;
        } else {
            value = value * 10 + digit;
        }
        *at += 1;
    }
    if (*at == start) {
        return -1;
    }
    if (too_large || value > XR_FMT_MAX_WIDTH) {
        char message[128];
        snprintf(message, sizeof(message), "%s is too large", what);
        xr_fmt_fail(message);
        return -2;
    }
    return value;
}

// Parse the template into literal and directive pieces. The array is sized for
// the worst case (every byte its own piece) and leaked like the rest of the
// bootstrap runtime.
static XrFmtPiece *xr_fmt_parse(XStr *template, long long *count_out) {
    const char *text = template->data;
    long long bytes = template->bytes;
    XrFmtPiece *pieces =
        (XrFmtPiece *)xr_alloc(sizeof(XrFmtPiece) * (size_t)(bytes + 1));
    long long count = 0;
    long long at = 0;
    long long literal_start = 0;
    while (at < bytes) {
        if (text[at] != '%') {
            at += 1;
            continue;
        }
        if (literal_start < at) {
            pieces[count].literal = 1;
            pieces[count].text = text + literal_start;
            pieces[count].bytes = at - literal_start;
            count += 1;
        }
        at += 1; // '%'
        if (at < bytes && text[at] == '%') {
            pieces[count].literal = 1;
            pieces[count].text = "%";
            pieces[count].bytes = 1;
            count += 1;
            at += 1;
            literal_start = at;
            continue;
        }
        int dash = 0;
        int plus = 0;
        int zero = 0;
        int space = 0;
        int alt = 0;
        while (at < bytes) {
            int repeated = 0;
            if (text[at] == '-') {
                repeated = dash;
                dash = 1;
            } else if (text[at] == '+') {
                repeated = plus;
                plus = 1;
            } else if (text[at] == '0') {
                repeated = zero;
                zero = 1;
            } else if (text[at] == ' ') {
                repeated = space;
                space = 1;
            } else if (text[at] == '#') {
                repeated = alt;
                alt = 1;
            } else {
                break;
            }
            if (repeated) {
                char message[96];
                snprintf(message, sizeof(message), "repeated flag `%c` in directive", text[at]);
                xr_fmt_fail(message);
            }
            at += 1;
        }
        long long width = xr_fmt_number(text, &at, bytes, "width");
        if (width == -2) {
            *count_out = count;
            return pieces;
        }
        long long precision = -1;
        if (at < bytes && text[at] == '.') {
            at += 1;
            long long value = xr_fmt_number(text, &at, bytes, "precision");
            if (value < 0) {
                xr_fmt_fail("`.` must be followed by precision digits");
                *count_out = count;
                return pieces;
            }
            precision = value;
        }
        if (at >= bytes) {
            xr_fmt_fail("incomplete directive: `%` with no verb");
            *count_out = count;
            return pieces;
        }
        char verb = text[at];
        if (verb == '%') {
            xr_fmt_fail("`%%` takes no flags, width, or precision");
            *count_out = count;
            return pieces;
        }
        if (!xr_fmt_is_verb(verb)) {
            char message[96];
            snprintf(message, sizeof(message), "unknown verb `%c`", verb);
            xr_fmt_fail(message);
            *count_out = count;
            return pieces;
        }
        at += 1;
        if ((plus || space) && !xr_fmt_is_numeric_verb(verb)) {
            char message[96];
            snprintf(message, sizeof(message), "flag `%c` is not valid for verb `%c`",
                     plus ? '+' : ' ', verb);
            xr_fmt_fail(message);
            *count_out = count;
            return pieces;
        }
        if (zero && !xr_fmt_is_numeric_verb(verb)) {
            char message[96];
            snprintf(message, sizeof(message), "flag `0` is not valid for verb `%c`", verb);
            xr_fmt_fail(message);
            *count_out = count;
            return pieces;
        }
        if (alt && !xr_fmt_is_alt_verb(verb)) {
            char message[96];
            snprintf(message, sizeof(message), "flag `#` is not valid for verb `%c`", verb);
            xr_fmt_fail(message);
            *count_out = count;
            return pieces;
        }
        if (precision >= 0) {
            long long lo = 0;
            long long hi = 0;
            int bounded = 1;
            if (verb == 'f' || verb == 'e' || verb == 'E') {
                lo = 0;
                hi = 15;
            } else if (verb == 'g' || verb == 'G') {
                lo = 1;
                hi = 17;
            } else if (verb == 'v' || verb == 's' || verb == 'q') {
                bounded = 0;
            } else {
                char message[96];
                snprintf(message, sizeof(message), "precision is not valid for verb `%c`", verb);
                xr_fmt_fail(message);
                *count_out = count;
                return pieces;
            }
            if (bounded && (precision < lo || precision > hi)) {
                char message[128];
                snprintf(message, sizeof(message),
                         "precision out of range for verb `%c`: %lld to %lld", verb, lo, hi);
                xr_fmt_fail(message);
                *count_out = count;
                return pieces;
            }
        }
        pieces[count].literal = 0;
        pieces[count].verb = verb;
        pieces[count].dash = dash;
        pieces[count].plus = plus;
        pieces[count].zero = zero;
        pieces[count].space = space;
        pieces[count].alt = alt;
        pieces[count].width = width < 0 ? -1 : width;
        pieces[count].precision = precision;
        count += 1;
        literal_start = at;
    }
    if (literal_start < bytes) {
        pieces[count].literal = 1;
        pieces[count].text = text + literal_start;
        pieces[count].bytes = bytes - literal_start;
        count += 1;
    }
    *count_out = count;
    return pieces;
}

static const char *xr_fmt_arg_kind(XVal value) {
    if (value.tag == XT_STR) {
        return "a string";
    }
    if (value.tag == XT_INT) {
        return "an int";
    }
    if (value.tag == XT_FLOAT) {
        return "a float";
    }
    if (value.tag == XT_BOOL) {
        return "a bool";
    }
    if (value.tag == XT_NIL) {
        return "null";
    }
    if (value.tag == XT_OBJ) {
        return "an object";
    }
    return "a rendered value";
}

// A whole float is its exact integer value; anything else is a mismatch.
static int xr_fmt_int_of(XVal value, long long *out) {
    if (value.tag == XT_INT) {
        *out = value.i;
        return 1;
    }
    if (value.tag == XT_FLOAT) {
        double f = value.f;
        if (isfinite(f) && f == trunc(f) && f >= -9223372036854775808.0 &&
            f < 9223372036854775808.0) {
            *out = (long long)f;
            return 1;
        }
    }
    return 0;
}

static int xr_fmt_float_of(XVal value, double *out) {
    if (value.tag == XT_INT) {
        *out = (double)value.i;
        return 1;
    }
    if (value.tag == XT_FLOAT) {
        *out = value.f;
        return 1;
    }
    return 0;
}

static void xr_fmt_append(char **out, long long *len, long long *cap, const char *text,
                          long long bytes) {
    xr_append_str(out, len, cap, text, bytes);
}

static void xr_fmt_fill(char **out, long long *len, long long *cap, char c, long long n) {
    for (long long i = 0; i < n; i += 1) {
        xr_append_str(out, len, cap, &c, 1);
    }
}

// Sign, `#` prefix, and body under the width rules: sign-aware zero padding,
// and `0` ignored when `-` is given (spec `modules/fmt.md`).
static void xr_fmt_assemble(char **out, long long *len, long long *cap, XrFmtPiece *piece,
                            const char *sign, const char *alt, const char *body,
                            long long body_bytes) {
    long long sign_bytes = (long long)strlen(sign);
    long long alt_bytes = (long long)strlen(alt);
    long long body_scalars = xr_utf8_count(body, body_bytes);
    long long total = sign_bytes + alt_bytes + body_scalars;
    long long width = piece->width < 0 ? 0 : piece->width;
    if (width > total) {
        long long fill = width - total;
        if (piece->dash) {
            xr_fmt_append(out, len, cap, sign, sign_bytes);
            xr_fmt_append(out, len, cap, alt, alt_bytes);
            xr_fmt_append(out, len, cap, body, body_bytes);
            xr_fmt_fill(out, len, cap, ' ', fill);
            return;
        }
        if (piece->zero && xr_fmt_is_numeric_verb(piece->verb)) {
            xr_fmt_append(out, len, cap, sign, sign_bytes);
            xr_fmt_append(out, len, cap, alt, alt_bytes);
            xr_fmt_fill(out, len, cap, '0', fill);
            xr_fmt_append(out, len, cap, body, body_bytes);
            return;
        }
        xr_fmt_fill(out, len, cap, ' ', fill);
    }
    xr_fmt_append(out, len, cap, sign, sign_bytes);
    xr_fmt_append(out, len, cap, alt, alt_bytes);
    xr_fmt_append(out, len, cap, body, body_bytes);
}

static const char *xr_fmt_sign(XrFmtPiece *piece, int negative) {
    if (negative) {
        return "-";
    }
    if (piece->plus) {
        return "+";
    }
    if (piece->space) {
        return " ";
    }
    return "";
}

static XVal xr_fmt_make_format(long long precision, int alternate) {
    const char *names[2];
    XVal values[2];
    names[0] = "precision";
    names[1] = "alternate";
    values[0] = precision >= 0 ? xr_int(precision) : xr_nil();
    values[1] = xr_bool(alternate);
    return xr_struct("Format", 2, names, values);
}

// `%v` / `%s`: consult `Display`, then `Debug`, for a user type. A derived
// `Debug` counts as a consulted implementation, so precision is not re-cut.
// Otherwise the table rendering is used and a written precision cuts it.
static void xr_fmt_text_of(XVal value, long long precision, int alternate, const char **body,
                           long long *body_bytes, int *consulted) {
    *body = "";
    *body_bytes = 0;
    *consulted = 0;
    if (value.tag == XT_OBJ) {
        XObj *object = xr_as_obj(value);
        int is_map = object->variant == NULL && strcmp(object->type, "Map") == 0;
        int is_struct = object->variant == NULL && !is_map;
        if (!is_map) {
            const char *which = NULL;
            if (xr_lookup_method(object->type, "display") != NULL) {
                which = "display";
            } else if (xr_lookup_method(object->type, "debug") != NULL) {
                which = "debug";
            }
            if (which != NULL) {
                XVal format = xr_fmt_make_format(precision, alternate);
                XVal rendered = xr_method(value, which, 1, &format);
                if (rendered.tag != XT_STR) {
                    xr_fmt_fail("`Display`/`Debug` must return a string");
                    return;
                }
                XStr *text = xr_as_str(rendered);
                *body = text->data;
                *body_bytes = text->bytes;
                *consulted = 1;
                return;
            }
        }
        if (is_struct && !object->debug) {
            xr_fmt_fail("`%v` and `%s` need `Display` or `Debug` for a struct");
            return;
        }
        if (is_struct && object->debug) {
            *consulted = 1;
        }
    }
    XVal rendered = xr_str_of(value);
    XStr *text = xr_as_str(rendered);
    *body = text->data;
    *body_bytes = text->bytes;
}

static void xr_fmt_quote(const char *text, long long bytes, char **out, long long *len,
                         long long *cap) {
    xr_append_str(out, len, cap, "\"", 1);
    for (long long at = 0; at < bytes; at += 1) {
        unsigned char c = (unsigned char)text[at];
        if (c == '"') {
            xr_append_str(out, len, cap, "\\\"", 2);
        } else if (c == '\\') {
            xr_append_str(out, len, cap, "\\\\", 2);
        } else if (c == '\n') {
            xr_append_str(out, len, cap, "\\n", 2);
        } else if (c == '\r') {
            xr_append_str(out, len, cap, "\\r", 2);
        } else if (c == '\t') {
            xr_append_str(out, len, cap, "\\t", 2);
        } else if (c < 0x20 || c == 0x7F) {
            char buffer[8];
            int n = snprintf(buffer, sizeof(buffer), "\\u%04x", c);
            xr_append_str(out, len, cap, buffer, n);
        } else {
            char one = (char)c;
            xr_append_str(out, len, cap, &one, 1);
        }
    }
    xr_append_str(out, len, cap, "\"", 1);
}

static void xr_fmt_unsigned(unsigned long long magnitude, int base, int upper, char **out,
                            long long *len, long long *cap) {
    char digits[72];
    int count = 0;
    if (magnitude == 0) {
        digits[count++] = '0';
    }
    while (magnitude > 0) {
        int digit = (int)(magnitude % (unsigned long long)base);
        digits[count++] = (char)(digit < 10 ? '0' + digit : (upper ? 'A' : 'a') + digit - 10);
        magnitude /= (unsigned long long)base;
    }
    for (int i = count - 1; i >= 0; i -= 1) {
        xr_append_str(out, len, cap, &digits[i], 1);
    }
}

// The shortest scientific digit string for `abs` (Rust's `{:e}`), then rounded
// to `sig` significant digits halfway away from zero. `digits` receives the
// significant digits and `exp` the decimal exponent of the leading one.
static void xr_fmt_round_sig(double abs, long long sig, int *digits, long long *count,
                             long long *exp) {
    if (abs == 0.0) {
        digits[0] = 0;
        *count = 1;
        *exp = 0;
        return;
    }
    char buffer[64];
    int precision = 1;
    for (; precision <= 17; precision += 1) {
        snprintf(buffer, sizeof(buffer), "%.*e", precision - 1, abs);
        if (strtod(buffer, NULL) == abs) {
            break;
        }
    }
    if (precision > 17) {
        snprintf(buffer, sizeof(buffer), "%.17e", abs);
    }
    long long n = 0;
    char *e = strchr(buffer, 'e');
    if (e != NULL) {
        *exp = atoll(e + 1);
    } else {
        *exp = 0;
    }
    for (char *p = buffer; *p != '\0' && *p != 'e'; p += 1) {
        if (*p >= '0' && *p <= '9') {
            digits[n++] = *p - '0';
        }
    }
    if (n > sig) {
        int round_up = digits[sig] >= 5;
        n = sig;
        if (round_up) {
            long long k = sig;
            while (k > 0 && digits[k - 1] == 9) {
                digits[k - 1] = 0;
                k -= 1;
            }
            if (k == 0) {
                for (long long i = 1; i < sig; i += 1) {
                    digits[i] = 0;
                }
                digits[0] = 1;
                *exp = *exp + 1;
            } else {
                digits[k - 1] += 1;
            }
        }
    }
    *count = n;
}

static void xr_fmt_pad_digits(int *digits, long long *count, long long n) {
    while (*count < n) {
        digits[*count] = 0;
        *count += 1;
    }
}

static void xr_fmt_trim_digits(int *digits, long long *count) {
    while (*count > 1 && digits[*count - 1] == 0) {
        *count -= 1;
    }
}

static void xr_fmt_fixed_form(int *digits, long long count, long long exp, char *out,
                              long long *out_len) {
    long long at = 0;
    if (exp >= 0) {
        long long int_len = exp + 1;
        if (count >= int_len) {
            for (long long i = 0; i < int_len; i += 1) {
                out[at++] = (char)('0' + digits[i]);
            }
            if (count > int_len) {
                out[at++] = '.';
                for (long long i = int_len; i < count; i += 1) {
                    out[at++] = (char)('0' + digits[i]);
                }
            }
        } else {
            for (long long i = 0; i < count; i += 1) {
                out[at++] = (char)('0' + digits[i]);
            }
            for (long long i = count; i < int_len; i += 1) {
                out[at++] = '0';
            }
        }
    } else {
        out[at++] = '0';
        out[at++] = '.';
        for (long long i = 0; i < -exp - 1; i += 1) {
            out[at++] = '0';
        }
        for (long long i = 0; i < count; i += 1) {
            out[at++] = (char)('0' + digits[i]);
        }
    }
    *out_len = at;
}

static void xr_fmt_trim_frac(char *text, long long *len) {
    int has_point = 0;
    for (long long i = 0; i < *len; i += 1) {
        if (text[i] == '.') {
            has_point = 1;
        }
    }
    if (!has_point) {
        return;
    }
    while (*len > 0 && text[*len - 1] == '0') {
        *len -= 1;
    }
    if (*len > 0 && text[*len - 1] == '.') {
        *len -= 1;
    }
}

static void xr_fmt_exponent(long long exp, char *out, long long *out_len) {
    long long at = 0;
    if (exp < 0) {
        out[at++] = '-';
        exp = -exp;
    } else {
        out[at++] = '+';
    }
    if (exp < 10) {
        out[at++] = '0';
    }
    char digits[24];
    int count = 0;
    if (exp == 0) {
        digits[count++] = '0';
    }
    while (exp > 0) {
        digits[count++] = (char)('0' + (int)(exp % 10));
        exp /= 10;
    }
    for (int i = count - 1; i >= 0; i -= 1) {
        out[at++] = digits[i];
    }
    *out_len = at;
}

static void xr_fmt_sci_form(int *digits, long long count, long long exp, int upper,
                            char *out, long long *out_len) {
    long long at = 0;
    out[at++] = (char)('0' + digits[0]);
    if (count > 1) {
        out[at++] = '.';
        for (long long i = 1; i < count; i += 1) {
            out[at++] = (char)('0' + digits[i]);
        }
    }
    out[at++] = upper ? 'E' : 'e';
    long long exp_len = 0;
    xr_fmt_exponent(exp, out + at, &exp_len);
    at += exp_len;
    *out_len = at;
}

static XVal xr_fmt_format(XVal template_value, XVal args_value) {
    if (template_value.tag != XT_STR) {
        xr_fmt_fail("expects a string template");
        return xr_str("");
    }
    XStr *template = xr_as_str(template_value);
    XVal args = args_value;
    if (args.tag == XT_NIL) {
        args = xr_list_new();
    }
    if (args.tag != XT_LIST) {
        xr_fmt_fail("expects a list for `args`");
        return xr_str("");
    }
    XList *list = xr_as_list(args);
    long long count = 0;
    XrFmtPiece *pieces = xr_fmt_parse(template, &count);
    long long directives = 0;
    for (long long i = 0; i < count; i += 1) {
        if (!pieces[i].literal) {
            directives += 1;
        }
    }
    if (directives != list->len) {
        char message[96];
        snprintf(message, sizeof(message), "expects %lld argument%s, found %lld", directives,
                 directives == 1 ? "" : "s", list->len);
        xr_fmt_fail(message);
        return xr_str("");
    }

    char *out = NULL;
    long long out_len = 0;
    long long out_cap = 0;
    long long next = 0;
    for (long long i = 0; i < count; i += 1) {
        XrFmtPiece *piece = &pieces[i];
        if (piece->literal) {
            xr_append_str(&out, &out_len, &out_cap, piece->text, piece->bytes);
            continue;
        }
        XVal arg = list->items[next];
        next += 1;
        char verb = piece->verb;
        if (verb == 'v' || verb == 's') {
            const char *body = NULL;
            long long body_bytes = 0;
            int consulted = 0;
            xr_fmt_text_of(arg, piece->precision, piece->alt, &body, &body_bytes, &consulted);
            if (!consulted && piece->precision >= 0) {
                body_bytes = xr_utf8_offset(body, body_bytes, piece->precision);
            }
            xr_fmt_assemble(&out, &out_len, &out_cap, piece, "", "", body, body_bytes);
            continue;
        }
        if (verb == 'q') {
            if (arg.tag != XT_STR) {
                char message[128];
                snprintf(message, sizeof(message), "`%%q` takes a string, found %s",
                         xr_fmt_arg_kind(arg));
                xr_fmt_fail(message);
                continue;
            }
            XStr *text = xr_as_str(arg);
            long long bytes = text->bytes;
            if (piece->precision >= 0) {
                bytes = xr_utf8_offset(text->data, bytes, piece->precision);
            }
            char *quoted = NULL;
            long long quoted_len = 0;
            long long quoted_cap = 0;
            xr_fmt_quote(text->data, bytes, &quoted, &quoted_len, &quoted_cap);
            xr_fmt_assemble(&out, &out_len, &out_cap, piece, "", "", quoted, quoted_len);
            continue;
        }
        if (verb == 't') {
            if (arg.tag != XT_BOOL) {
                char message[128];
                snprintf(message, sizeof(message), "`%%t` takes a bool value, found %s",
                         xr_fmt_arg_kind(arg));
                xr_fmt_fail(message);
                continue;
            }
            const char *body = arg.i != 0 ? "true" : "false";
            xr_fmt_assemble(&out, &out_len, &out_cap, piece, "", "", body,
                            (long long)strlen(body));
            continue;
        }
        if (verb == 'c') {
            long long n = 0;
            if (!xr_fmt_int_of(arg, &n)) {
                char message[128];
                snprintf(message, sizeof(message), "`%%c` takes an int value, found %s",
                         xr_fmt_arg_kind(arg));
                xr_fmt_fail(message);
                continue;
            }
            if (n < 0 || n > 0x10FFFF || (n >= 0xD800 && n <= 0xDFFF)) {
                char message[128];
                snprintf(message, sizeof(message),
                         "`%%c` scalar out of range: %lld is not a valid scalar", n);
                xr_fmt_fail(message);
                continue;
            }
            char body[4];
            int bytes = 0;
            xr_utf8_encode((unsigned int)n, body, &bytes);
            xr_fmt_assemble(&out, &out_len, &out_cap, piece, "", "", body, bytes);
            continue;
        }
        if (verb == 'd' || verb == 'x' || verb == 'X' || verb == 'o' || verb == 'b') {
            long long n = 0;
            if (!xr_fmt_int_of(arg, &n)) {
                char message[128];
                snprintf(message, sizeof(message), "`%%%c` takes an int value, found %s", verb,
                         xr_fmt_arg_kind(arg));
                xr_fmt_fail(message);
                continue;
            }
            int negative = n < 0;
            unsigned long long magnitude = negative
                                               ? (unsigned long long)(-(n + 1)) + 1ULL
                                               : (unsigned long long)n;
            int base = verb == 'o' ? 8 : (verb == 'b' ? 2 : 10);
            int upper = verb == 'X';
            if (verb == 'x' || verb == 'X') {
                base = 16;
            }
            char *body = NULL;
            long long body_bytes = 0;
            long long body_cap = 0;
            xr_fmt_unsigned(magnitude, base, upper, &body, &body_bytes, &body_cap);
            const char *alt = "";
            if (piece->alt) {
                if (verb == 'x') {
                    alt = "0x";
                } else if (verb == 'X') {
                    alt = "0X";
                } else if (verb == 'o') {
                    alt = "0o";
                } else if (verb == 'b') {
                    alt = "0b";
                }
            }
            const char *sign = xr_fmt_sign(piece, negative);
            xr_fmt_assemble(&out, &out_len, &out_cap, piece, sign, alt, body, body_bytes);
            continue;
        }
        // Floating-point verbs.
        double value = 0.0;
        if (!xr_fmt_float_of(arg, &value)) {
            char message[128];
            snprintf(message, sizeof(message), "`%%%c` takes a number value, found %s", verb,
                     xr_fmt_arg_kind(arg));
            xr_fmt_fail(message);
            continue;
        }
        if (isnan(value) || isinf(value)) {
            XVal special = xr_format_float(value);
            XStr *text = xr_as_str(special);
            long long width = piece->width < 0 ? 0 : piece->width;
            if (width > text->len) {
                long long fill = width - text->len;
                if (!piece->dash) {
                    xr_fmt_fill(&out, &out_len, &out_cap, ' ', fill);
                }
                xr_append_str(&out, &out_len, &out_cap, text->data, text->bytes);
                if (piece->dash) {
                    xr_fmt_fill(&out, &out_len, &out_cap, ' ', fill);
                }
            } else {
                xr_append_str(&out, &out_len, &out_cap, text->data, text->bytes);
            }
            continue;
        }
        int negative = signbit(value) != 0;
        double abs = fabs(value);
        char body_buffer[512];
        long long body_len = 0;
        if (verb == 'f') {
            long long precision = piece->precision >= 0 ? piece->precision : 6;
            XVal text = xr_math_to_fixed(xr_float(abs), xr_int(precision));
            XStr *string = xr_as_str(text);
            long long copy = string->bytes < (long long)sizeof(body_buffer)
                                 ? string->bytes
                                 : (long long)sizeof(body_buffer);
            memcpy(body_buffer, string->data, (size_t)copy);
            body_len = copy;
        } else if (verb == 'e' || verb == 'E') {
            long long precision = piece->precision >= 0 ? piece->precision : 6;
            int digits[32];
            long long count = 0;
            long long exp = 0;
            xr_fmt_round_sig(abs, precision + 1, digits, &count, &exp);
            xr_fmt_pad_digits(digits, &count, precision + 1);
            xr_fmt_sci_form(digits, count, exp, verb == 'E', body_buffer, &body_len);
        } else {
            int upper = verb == 'G';
            int digits[32];
            long long count = 0;
            long long exp = 0;
            if (piece->precision >= 0) {
                long long precision = piece->precision;
                xr_fmt_round_sig(abs, precision, digits, &count, &exp);
                xr_fmt_trim_digits(digits, &count);
                if (exp >= -4 && exp < precision) {
                    xr_fmt_fixed_form(digits, count, exp, body_buffer, &body_len);
                    xr_fmt_trim_frac(body_buffer, &body_len);
                } else {
                    xr_fmt_sci_form(digits, count, exp, upper, body_buffer, &body_len);
                }
            } else {
                xr_fmt_round_sig(abs, 17, digits, &count, &exp);
                if (exp >= -4 && exp < 6) {
                    xr_fmt_fixed_form(digits, count, exp, body_buffer, &body_len);
                    xr_fmt_trim_frac(body_buffer, &body_len);
                } else {
                    xr_fmt_sci_form(digits, count, exp, upper, body_buffer, &body_len);
                }
            }
        }
        const char *sign = xr_fmt_sign(piece, negative);
        xr_fmt_assemble(&out, &out_len, &out_cap, piece, sign, "", body_buffer, body_len);
    }
    return xr_str_n(out == NULL ? "" : out, out_len);
}
