// ── StringBuilder ────────────────────────────────────────────────────────

static XVal xr_sb_new(void) {
    XVal v = xr_mk(XT_SB);
    XSb *sb = (XSb *)xr_alloc(sizeof(XSb));
    sb->len = 0;
    sb->bytes = 0;
    sb->data = (char *)xr_alloc(1);
    v.p = sb;
    return v;
}

static void xr_sb_append(XVal v, XVal text) {
    if (v.tag != XT_SB || text.tag != XT_STR) {
        xr_fail("`.append` expects a StringBuilder and a string");
    }
    XSb *sb = (XSb *)v.p;
    XStr *s = xr_as_str(text);
    long long cap = sb->bytes + s->bytes + 1;
    char *data = (char *)xr_alloc((size_t)cap);
    if (sb->bytes > 0) {
        memcpy(data, sb->data, (size_t)sb->bytes);
    }
    memcpy(data + sb->bytes, s->data, (size_t)s->bytes);
    data[cap - 1] = '\0';
    sb->data = data;
    sb->bytes += s->bytes;
    sb->len += s->len;
}

// ── methods ──────────────────────────────────────────────────────────────

typedef struct XrMethodReg {
    const char *type;
    const char *name;
    XrMethodFn fn;
    struct XrMethodReg *next;
} XrMethodReg;

static XrMethodReg *xr_method_registry = NULL;

void xr_register_method(const char *type, const char *name, XrMethodFn fn) {
    XrMethodReg *entry = (XrMethodReg *)xr_alloc(sizeof(XrMethodReg));
    entry->type = type;
    entry->name = name;
    entry->fn = fn;
    entry->next = xr_method_registry;
    xr_method_registry = entry;
}

static XrMethodFn xr_lookup_method(const char *type, const char *name) {
    XrMethodReg *entry = xr_method_registry;
    while (entry != NULL) {
        if (strcmp(entry->type, type) == 0 && strcmp(entry->name, name) == 0) {
            return entry->fn;
        }
        entry = entry->next;
    }
    return NULL;
}

static XVal xr_key_string(XVal key) {
    // Map keys follow the stage 0 rule: a string key stays as written and
    // every other hashable value is rendered first (spec
    // `types/composite-types.md`).
    if (key.tag == XT_STR) {
        return key;
    }
    return xr_str_of(key);
}

static XVal *xr_args_with_self(XVal self, long long n, XVal *args) {
    XVal *all = (XVal *)xr_alloc(sizeof(XVal) * (size_t)(n + 1));
    all[0] = self;
    for (long long i = 0; i < n; i += 1) {
        all[i + 1] = args[i];
    }
    return all;
}

XVal xr_method(XVal recv, const char *name, long long n, XVal *args) {
    if (recv.tag == XT_OBJ) {
        XObj *o = xr_as_obj(recv);
        if (strcmp(o->type, "RegExp") == 0) {
            return xr_regex_method(recv, name, n, args);
        }
        if (strcmp(o->type, "RegexMatch") == 0) {
            return xr_regex_match_method(recv, name, n, args);
        }
        XrMethodFn fn = xr_lookup_method(o->type, name);
        if (fn != NULL) {
            return fn(n + 1, xr_args_with_self(recv, n, args));
        }
        // Synchronous service model: `send` on a type with `on_message`
        // invokes the handler and returns its `reply`; `stop` is a no-op.
        if (o->variant == NULL && xr_lookup_method(o->type, "on_message") != NULL) {
            if (strcmp(name, "send") == 0) {
                XrMethodFn handler = xr_lookup_method(o->type, "on_message");
                return handler(n + 1, xr_args_with_self(recv, n, args));
            }
            if (strcmp(name, "stop") == 0) {
                return xr_nil();
            }
        }
        if (o->variant == NULL && strcmp(o->type, "Map") == 0) {
            if (strcmp(name, "get") == 0 || strcmp(name, "has") == 0
                || strcmp(name, "remove") == 0) {
                if (n != 1) {
                    xr_fail("map method expects one key argument");
                }
                if (strcmp(name, "get") == 0) {
                    return xr_map_get(recv, args[0]);
                }
                if (strcmp(name, "has") == 0) {
                    return xr_bool(xr_map_has(recv, args[0]));
                }
                return xr_map_remove(recv, args[0]);
            }
            if (strcmp(name, "set") == 0) {
                if (n != 2) {
                    xr_fail("`.set` expects a key and a value");
                }
                xr_map_set(recv, args[0], args[1]);
                return xr_nil();
            }
            if (strcmp(name, "keys") == 0 || strcmp(name, "values") == 0
                || strcmp(name, "entries") == 0) {
                if (n != 0) {
                    xr_fail("map projection expects no arguments");
                }
                XVal out = xr_list_new();
                for (long long i = 0; i < o->nfields; i += 1) {
                    if (strcmp(name, "keys") == 0) {
                        xr_list_push(out, xr_str(o->fields[i].name));
                    } else if (strcmp(name, "values") == 0) {
                        xr_list_push(out, o->fields[i].value);
                    } else {
                        XVal pair = xr_list_new();
                        xr_list_push(pair, xr_str(o->fields[i].name));
                        xr_list_push(pair, o->fields[i].value);
                        xr_list_push(out, pair);
                    }
                }
                return out;
            }
            xr_fail("unknown map method");
        }
    }
    if (recv.tag == XT_SET) {
        XSet *set = (XSet *)recv.p;
        if (strcmp(name, "has") == 0 || strcmp(name, "insert") == 0
            || strcmp(name, "remove") == 0) {
            if (n != 1) {
                xr_fail("set method expects one value argument");
            }
            if (strcmp(name, "has") == 0) {
                return xr_bool(xr_set_find(set, args[0]) >= 0);
            }
            if (strcmp(name, "insert") == 0) {
                return xr_bool(xr_set_insert(set, args[0]));
            }
            return xr_bool(xr_set_remove(set, args[0]));
        }
        xr_fail("unknown set method");
    }
    if (strcmp(name, "slice") == 0 && recv.tag == XT_STR) {
        long long start = n > 0 ? args[0].i : 0;
        long long end = n > 1 ? args[1].i : xr_as_str(recv)->len;
        if (n > 1 && args[1].tag == XT_NIL) {
            end = xr_as_str(recv)->len;
        }
        return xr_str_slice(recv, start, end);
    }
    if (strcmp(name, "code_at") == 0 && recv.tag == XT_STR) {
        return xr_str_code_at(recv, args[0].i);
    }
    if (strcmp(name, "bytes") == 0 && recv.tag == XT_STR) {
        XStr *s = xr_as_str(recv);
        XVal out = xr_list_new();
        for (long long i = 0; i < s->bytes; i += 1) {
            xr_list_push(out, xr_int((unsigned char)s->data[i]));
        }
        return out;
    }
    if (strcmp(name, "replace") == 0 && recv.tag == XT_STR) {
        if (n != 2) {
            xr_fail("`.replace` expects a pattern and a replacer");
        }
        return xr_regex_replace(args[0], recv, args[1]);
    }
    if (strcmp(name, "split") == 0 && recv.tag == XT_STR) {
        if (n == 1 && args[0].tag == XT_OBJ && strcmp(xr_as_obj(args[0])->type, "RegExp") == 0) {
            return xr_regex_split(args[0], recv);
        }
        XStr *s = xr_as_str(recv);
        XVal out = xr_list_new();
        if (args[0].tag != XT_STR) {
            xr_fail("`.split` expects a string separator");
        }
        XStr *sep = xr_as_str(args[0]);
        if (sep->bytes == 0) {
            long long at = 0;
            while (at < s->bytes) {
                long long step = xr_utf8_len((const unsigned char *)s->data + at);
                if (at + step > s->bytes) {
                    step = 1;
                }
                xr_list_push(out, xr_str_n(s->data + at, step));
                at += step;
            }
            return out;
        }
        long long start = 0;
        long long at = 0;
        while (at + sep->bytes <= s->bytes) {
            if (memcmp(s->data + at, sep->data, (size_t)sep->bytes) == 0) {
                xr_list_push(out, xr_str_n(s->data + start, at - start));
                at += sep->bytes;
                start = at;
            } else {
                at += 1;
            }
        }
        xr_list_push(out, xr_str_n(s->data + start, s->bytes - start));
        return out;
    }
    if (strcmp(name, "starts_with") == 0 && recv.tag == XT_STR) {
        return xr_bool(xr_str_starts_with(recv, args[0]));
    }
    if (strcmp(name, "ends_with") == 0 && recv.tag == XT_STR) {
        return xr_bool(xr_str_ends_with(recv, args[0]));
    }
    if (strcmp(name, "contains") == 0 && recv.tag == XT_STR) {
        return xr_bool(xr_str_contains(recv, args[0]));
    }
    if (recv.tag == XT_LIST) {
        XList *l = xr_as_list(recv);
        if (strcmp(name, "push") == 0) {
            xr_list_push(recv, args[0]);
            return xr_nil();
        }
        if (strcmp(name, "clear") == 0) {
            l->len = 0;
            return xr_nil();
        }
        if (strcmp(name, "slice") == 0) {
            long long start = n > 0 ? args[0].i : 0;
            long long end = n > 1 ? args[1].i : l->len;
            if (start < 0) {
                start = 0;
            }
            if (end > l->len) {
                end = l->len;
            }
            XVal out = xr_list_new();
            for (long long i = start; i < end; i += 1) {
                xr_list_push(out, l->items[i]);
            }
            return out;
        }
        if (strcmp(name, "index_of") == 0) {
            for (long long i = 0; i < l->len; i += 1) {
                if (xr_eq(l->items[i], args[0])) {
                    return xr_int(i);
                }
            }
            return xr_int(-1);
        }
        if (strcmp(name, "includes") == 0) {
            for (long long i = 0; i < l->len; i += 1) {
                if (xr_eq(l->items[i], args[0])) {
                    return xr_bool(1);
                }
            }
            return xr_bool(0);
        }
        if (strcmp(name, "join") == 0) {
            XVal out = xr_sb_new();
            XVal sep = args[0];
            if (sep.tag != XT_STR) {
                xr_fail("`.join` expects a string separator");
            }
            for (long long i = 0; i < l->len; i += 1) {
                if (i > 0) {
                    xr_sb_append(out, sep);
                }
                xr_sb_append(out, xr_str_of(l->items[i]));
            }
            XSb *sb = (XSb *)out.p;
            return xr_str_n(sb->data == NULL ? "" : sb->data, sb->bytes);
        }
    }
    if (recv.tag == XT_SB) {
        if (strcmp(name, "append") == 0) {
            xr_sb_append(recv, args[0]);
            return xr_nil();
        }
        if (strcmp(name, "build") == 0) {
            XSb *sb = (XSb *)recv.p;
            return xr_str_n(sb->data == NULL ? "" : sb->data, sb->bytes);
        }
        if (strcmp(name, "clear") == 0) {
            XSb *sb = (XSb *)recv.p;
            sb->len = 0;
            sb->bytes = 0;
            sb->data = (char *)xr_alloc(1);
            return xr_nil();
        }
    }
    xr_fail("unknown method");
    return xr_nil();
}

XVal xr_static_method(const char *type, const char *name, long long n, XVal *args) {
    if (strcmp(type, "StringBuilder") == 0 && strcmp(name, "new") == 0) {
        return xr_sb_new();
    }
    if (strcmp(type, "Set") == 0 && strcmp(name, "from_list") == 0) {
        if (n != 1) {
            xr_fail("`Set::from_list` expects one argument");
        }
        return xr_set_from_list(args[0]);
    }
    if (strcmp(type, "Map") == 0 && strcmp(name, "from_entries") == 0) {
        if (n != 1) {
            xr_fail("`Map::from_entries` expects one argument");
        }
        return xr_map_from_entries(args[0]);
    }
    xr_fail("unknown static method");
    return xr_nil();
}
