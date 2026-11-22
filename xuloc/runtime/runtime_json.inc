// ── std::json ────────────────────────────────────────────────────────────

#define XR_JSON_MAX_DEPTH 128

typedef struct {
    const char *data;
    long long len;
    long long at;
    int error;
} XrJsonParser;

typedef struct {
    char *data;
    long long len;
    long long cap;
    int indent;
    int ok;
} XrJsonWriter;

static void xr_json_skip_ws(XrJsonParser *p) {
    while (p->at < p->len) {
        char c = p->data[p->at];
        if (c == ' ' || c == '\t' || c == '\n' || c == '\r') {
            p->at += 1;
        } else {
            break;
        }
    }
}

static int xr_json_hex4(XrJsonParser *p, unsigned int *out) {
    if (p->at + 4 > p->len) {
        return 0;
    }
    unsigned int value = 0;
    for (int i = 0; i < 4; i += 1) {
        char c = p->data[p->at + i];
        unsigned int digit;
        if (c >= '0' && c <= '9') {
            digit = (unsigned int)(c - '0');
        } else if (c >= 'a' && c <= 'f') {
            digit = (unsigned int)(c - 'a' + 10);
        } else if (c >= 'A' && c <= 'F') {
            digit = (unsigned int)(c - 'A' + 10);
        } else {
            return 0;
        }
        value = value * 16 + digit;
    }
    p->at += 4;
    *out = value;
    return 1;
}

static void xr_json_put(XrJsonWriter *w, const char *data, long long bytes) {
    xr_append_str(&w->data, &w->len, &w->cap, data, bytes);
}

static void xr_json_putc(XrJsonWriter *w, char c) { xr_json_put(w, &c, 1); }

static void xr_json_newline(XrJsonWriter *w, int depth) {
    if (w->indent <= 0) {
        return;
    }
    xr_json_putc(w, '\n');
    long long spaces = (long long)w->indent * (long long)depth;
    for (long long i = 0; i < spaces; i += 1) {
        xr_json_putc(w, ' ');
    }
}

static XVal xr_json_string(XrJsonParser *p) {
    // `p->at` points at the opening quote.
    p->at += 1;
    long long cap = 64;
    long long len = 0;
    char *buffer = (char *)xr_alloc((size_t)cap);
    while (p->at < p->len) {
        unsigned char c = (unsigned char)p->data[p->at];
        if (c == '"') {
            p->at += 1;
            return xr_str_n(buffer, len);
        }
        if (c < 0x20) {
            p->error = 1;
            return xr_nil();
        }
        if (c != '\\') {
            if (len + 2 > cap) {
                cap *= 2;
                char *next = (char *)xr_alloc((size_t)cap);
                memcpy(next, buffer, (size_t)len);
                buffer = next;
            }
            buffer[len] = (char)c;
            len += 1;
            p->at += 1;
            continue;
        }
        p->at += 1;
        if (p->at >= p->len) {
            p->error = 1;
            return xr_nil();
        }
        char escape = p->data[p->at];
        p->at += 1;
        char decoded = 0;
        if (escape == '"' || escape == '\\' || escape == '/') {
            decoded = escape;
        } else if (escape == 'b') {
            decoded = '\b';
        } else if (escape == 'f') {
            decoded = '\f';
        } else if (escape == 'n') {
            decoded = '\n';
        } else if (escape == 'r') {
            decoded = '\r';
        } else if (escape == 't') {
            decoded = '\t';
        }
        if (decoded != 0) {
            if (len + 2 > cap) {
                cap *= 2;
                char *next = (char *)xr_alloc((size_t)cap);
                memcpy(next, buffer, (size_t)len);
                buffer = next;
            }
            buffer[len] = decoded;
            len += 1;
            continue;
        }
        if (escape != 'u') {
            p->error = 1;
            return xr_nil();
        }
        unsigned int code = 0;
        if (!xr_json_hex4(p, &code)) {
            p->error = 1;
            return xr_nil();
        }
        if (code >= 0xD800 && code <= 0xDBFF) {
            if (p->at + 2 > p->len || p->data[p->at] != '\\' || p->data[p->at + 1] != 'u') {
                p->error = 1;
                return xr_nil();
            }
            p->at += 2;
            unsigned int low = 0;
            if (!xr_json_hex4(p, &low) || low < 0xDC00 || low > 0xDFFF) {
                p->error = 1;
                return xr_nil();
            }
            code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
        }
        char encoded[4];
        int bytes = 0;
        xr_utf8_encode(code, encoded, &bytes);
        if (len + bytes + 1 > cap) {
            while (len + bytes + 1 > cap) {
                cap *= 2;
            }
            char *next = (char *)xr_alloc((size_t)cap);
            memcpy(next, buffer, (size_t)len);
            buffer = next;
        }
        for (int i = 0; i < bytes; i += 1) {
            buffer[len] = encoded[i];
            len += 1;
        }
    }
    p->error = 1;
    return xr_nil();
}

static XVal xr_json_value(XrJsonParser *p, int depth);

static XVal xr_json_object(XrJsonParser *p, int depth) {
    if (depth >= XR_JSON_MAX_DEPTH) {
        p->error = 1;
        return xr_nil();
    }
    XVal map = xr_map_new();
    p->at += 1; // '{'
    xr_json_skip_ws(p);
    if (p->at < p->len && p->data[p->at] == '}') {
        p->at += 1;
        return map;
    }
    while (p->at < p->len) {
        xr_json_skip_ws(p);
        if (p->at >= p->len || p->data[p->at] != '"') {
            p->error = 1;
            return xr_nil();
        }
        XVal key = xr_json_string(p);
        if (p->error) {
            return xr_nil();
        }
        xr_json_skip_ws(p);
        if (p->at >= p->len || p->data[p->at] != ':') {
            p->error = 1;
            return xr_nil();
        }
        p->at += 1;
        XVal value = xr_json_value(p, depth + 1);
        if (p->error) {
            return xr_nil();
        }
        xr_map_set(map, key, value);
        xr_json_skip_ws(p);
        if (p->at < p->len && p->data[p->at] == ',') {
            p->at += 1;
            continue;
        }
        if (p->at < p->len && p->data[p->at] == '}') {
            p->at += 1;
            return map;
        }
        p->error = 1;
        return xr_nil();
    }
    p->error = 1;
    return xr_nil();
}

static XVal xr_json_array(XrJsonParser *p, int depth) {
    if (depth >= XR_JSON_MAX_DEPTH) {
        p->error = 1;
        return xr_nil();
    }
    XVal list = xr_list_new();
    p->at += 1; // '['
    xr_json_skip_ws(p);
    if (p->at < p->len && p->data[p->at] == ']') {
        p->at += 1;
        return list;
    }
    while (p->at < p->len) {
        XVal value = xr_json_value(p, depth + 1);
        if (p->error) {
            return xr_nil();
        }
        xr_list_push(list, value);
        xr_json_skip_ws(p);
        if (p->at < p->len && p->data[p->at] == ',') {
            p->at += 1;
            continue;
        }
        if (p->at < p->len && p->data[p->at] == ']') {
            p->at += 1;
            return list;
        }
        p->error = 1;
        return xr_nil();
    }
    p->error = 1;
    return xr_nil();
}

static XVal xr_json_number(XrJsonParser *p) {
    long long start = p->at;
    if (p->at < p->len && p->data[p->at] == '-') {
        p->at += 1;
    }
    if (p->at >= p->len) {
        p->error = 1;
        return xr_nil();
    }
    if (p->data[p->at] == '0') {
        p->at += 1;
    } else if (p->data[p->at] >= '1' && p->data[p->at] <= '9') {
        while (p->at < p->len && p->data[p->at] >= '0' && p->data[p->at] <= '9') {
            p->at += 1;
        }
    } else {
        p->error = 1;
        return xr_nil();
    }
    if (p->at < p->len && p->data[p->at] == '.') {
        p->at += 1;
        if (p->at >= p->len || p->data[p->at] < '0' || p->data[p->at] > '9') {
            p->error = 1;
            return xr_nil();
        }
        while (p->at < p->len && p->data[p->at] >= '0' && p->data[p->at] <= '9') {
            p->at += 1;
        }
    }
    if (p->at < p->len && (p->data[p->at] == 'e' || p->data[p->at] == 'E')) {
        p->at += 1;
        if (p->at < p->len && (p->data[p->at] == '+' || p->data[p->at] == '-')) {
            p->at += 1;
        }
        if (p->at >= p->len || p->data[p->at] < '0' || p->data[p->at] > '9') {
            p->error = 1;
            return xr_nil();
        }
        while (p->at < p->len && p->data[p->at] >= '0' && p->data[p->at] <= '9') {
            p->at += 1;
        }
    }
    long long size = p->at - start;
    char *text = (char *)xr_alloc((size_t)size + 1);
    memcpy(text, p->data + start, (size_t)size);
    text[size] = '\0';
    double value = strtod(text, NULL);
    if (isinf(value) || isnan(value)) {
        p->error = 1;
        return xr_nil();
    }
    return xr_float(value);
}

static int xr_json_literal(XrJsonParser *p, const char *text) {
    long long size = (long long)strlen(text);
    if (p->at + size > p->len || memcmp(p->data + p->at, text, (size_t)size) != 0) {
        return 0;
    }
    p->at += size;
    return 1;
}

static XVal xr_json_value(XrJsonParser *p, int depth) {
    xr_json_skip_ws(p);
    if (p->at >= p->len) {
        p->error = 1;
        return xr_nil();
    }
    char c = p->data[p->at];
    if (c == '{') {
        return xr_json_object(p, depth);
    }
    if (c == '[') {
        return xr_json_array(p, depth);
    }
    if (c == '"') {
        return xr_json_string(p);
    }
    if (c == 't' && xr_json_literal(p, "true")) {
        return xr_bool(1);
    }
    if (c == 'f' && xr_json_literal(p, "false")) {
        return xr_bool(0);
    }
    if (c == 'n' && xr_json_literal(p, "null")) {
        return xr_nil();
    }
    if (c == '-' || (c >= '0' && c <= '9')) {
        return xr_json_number(p);
    }
    p->error = 1;
    return xr_nil();
}

static XVal xr_json_decode(XVal text) {
    if (text.tag != XT_STR) {
        return xr_err("std::json::parse expects a string");
    }
    XStr *s = xr_as_str(text);
    XrJsonParser p;
    p.data = s->data;
    p.len = s->bytes;
    p.at = 0;
    p.error = 0;
    XVal value = xr_json_value(&p, 0);
    xr_json_skip_ws(&p);
    if (p.error || p.at != p.len) {
        return xr_err("invalid JSON");
    }
    return xr_ok(value);
}

XVal xr_json_parse(XVal text) { return xr_json_decode(text); }

XVal xr_json_valid(XVal text) {
    XVal result = xr_json_decode(text);
    if (result.tag != XT_OBJ) {
        return xr_bool(0);
    }
    XObj *o = xr_as_obj(result);
    return xr_bool(o->variant != NULL && strcmp(o->variant, "Ok") == 0);
}

static void xr_json_write(XrJsonWriter *w, XVal value, int depth);

static void xr_json_write_string(XrJsonWriter *w, const char *data, long long bytes) {
    xr_json_putc(w, '"');
    for (long long i = 0; i < bytes; i += 1) {
        unsigned char c = (unsigned char)data[i];
        if (c == '"') {
            xr_json_put(w, "\\\"", 2);
        } else if (c == '\\') {
            xr_json_put(w, "\\\\", 2);
        } else if (c == '\n') {
            xr_json_put(w, "\\n", 2);
        } else if (c == '\r') {
            xr_json_put(w, "\\r", 2);
        } else if (c == '\t') {
            xr_json_put(w, "\\t", 2);
        } else if (c == '\b') {
            xr_json_put(w, "\\b", 2);
        } else if (c == '\f') {
            xr_json_put(w, "\\f", 2);
        } else if (c < 0x20) {
            char buffer[7];
            snprintf(buffer, sizeof(buffer), "\\u%04X", c);
            xr_json_put(w, buffer, 6);
        } else {
            char byte = (char)c;
            xr_json_put(w, &byte, 1);
        }
    }
    xr_json_putc(w, '"');
}

static void xr_json_write(XrJsonWriter *w, XVal value, int depth) {
    if (!w->ok) {
        return;
    }
    if (value.tag == XT_NIL) {
        xr_json_put(w, "null", 4);
        return;
    }
    if (value.tag == XT_BOOL) {
        if (value.i != 0) {
            xr_json_put(w, "true", 4);
        } else {
            xr_json_put(w, "false", 5);
        }
        return;
    }
    if (value.tag == XT_INT) {
        char buffer[32];
        int size = snprintf(buffer, sizeof(buffer), "%lld", value.i);
        xr_json_put(w, buffer, (long long)size);
        return;
    }
    if (value.tag == XT_FLOAT) {
        if (isnan(value.f) || isinf(value.f)) {
            w->ok = 0;
            return;
        }
        XVal text = xr_format_float(value.f);
        XStr *s = xr_as_str(text);
        xr_json_put(w, s->data, s->bytes);
        return;
    }
    if (value.tag == XT_STR) {
        XStr *s = xr_as_str(value);
        xr_json_write_string(w, s->data, s->bytes);
        return;
    }
    if (value.tag == XT_LIST) {
        XList *list = xr_as_list(value);
        xr_json_putc(w, '[');
        for (long long i = 0; i < list->len; i += 1) {
            if (i > 0) {
                xr_json_putc(w, ',');
            }
            xr_json_newline(w, depth + 1);
            xr_json_write(w, list->items[i], depth + 1);
        }
        if (list->len > 0) {
            xr_json_newline(w, depth);
        }
        xr_json_putc(w, ']');
        return;
    }
    if (value.tag == XT_OBJ) {
        XObj *o = xr_as_obj(value);
        if (o->variant != NULL) {
            w->ok = 0;
            return;
        }
        long long count = o->nfields;
        long long *order = NULL;
        if (count > 0) {
            order = (long long *)xr_alloc(sizeof(long long) * (size_t)count);
            for (long long i = 0; i < count; i += 1) {
                order[i] = i;
            }
            // Members are emitted in UTF-8 byte order.
            for (long long i = 1; i < count; i += 1) {
                long long at = order[i];
                long long j = i;
                while (j > 0 && strcmp(o->fields[order[j - 1]].name, o->fields[at].name) > 0) {
                    order[j] = order[j - 1];
                    j -= 1;
                }
                order[j] = at;
            }
        }
        xr_json_putc(w, '{');
        for (long long i = 0; i < count; i += 1) {
            if (i > 0) {
                xr_json_putc(w, ',');
            }
            xr_json_newline(w, depth + 1);
            const char *name = o->fields[order[i]].name;
            xr_json_write_string(w, name, (long long)strlen(name));
            if (w->indent > 0) {
                xr_json_put(w, ": ", 2);
            } else {
                xr_json_putc(w, ':');
            }
            xr_json_write(w, o->fields[order[i]].value, depth + 1);
        }
        if (count > 0) {
            xr_json_newline(w, depth);
        }
        xr_json_putc(w, '}');
        return;
    }
    w->ok = 0;
}

XVal xr_json_stringify(XVal value, XVal indent) {
    long long spaces = 0;
    if (indent.tag != XT_NIL) {
        double amount;
        if (indent.tag == XT_INT) {
            amount = (double)indent.i;
        } else if (indent.tag == XT_FLOAT) {
            amount = indent.f;
        } else {
            return xr_err("indent outside 0..=8");
        }
        if (isnan(amount) || isinf(amount) || amount < 0.0 || amount > 8.0) {
            return xr_err("indent outside 0..=8");
        }
        spaces = (long long)amount; // fractional values truncate toward zero
    }
    XrJsonWriter w;
    w.data = NULL;
    w.len = 0;
    w.cap = 0;
    w.indent = (int)spaces;
    w.ok = 1;
    xr_json_write(&w, value, 0);
    if (!w.ok) {
        return xr_err("cannot serialize value as JSON");
    }
    return xr_ok(xr_str_n(w.data == NULL ? "" : w.data, w.len));
}

