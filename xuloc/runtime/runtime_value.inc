// ── allocation ───────────────────────────────────────────────────────────

static void *xr_alloc(size_t n) {
    void *p = calloc(1, n == 0 ? 1 : n);
    if (p == NULL) {
        fputs("xuloc runtime: out of memory\n", stderr);
        exit(70);
    }
    return p;
}

static char *xr_copy_bytes(const char *data, long long bytes) {
    char *out = (char *)xr_alloc((size_t)bytes + 1);
    if (bytes > 0) {
        memcpy(out, data, (size_t)bytes);
    }
    out[bytes] = '\0';
    return out;
}

// ── UTF-8 ────────────────────────────────────────────────────────────────

// Length in bytes of the scalar starting at `s` (at least one byte).
static long long xr_utf8_len(const unsigned char *s) {
    unsigned char c = s[0];
    if (c < 0x80) {
        return 1;
    }
    if ((c & 0xE0) == 0xC0) {
        return 2;
    }
    if ((c & 0xF0) == 0xE0) {
        return 3;
    }
    if ((c & 0xF8) == 0xF0) {
        return 4;
    }
    return 1;
}

static unsigned int xr_utf8_decode(const unsigned char *s, long long bytes) {
    if (bytes <= 0) {
        return 0;
    }
    unsigned char c = s[0];
    if (c < 0x80) {
        return c;
    }
    if ((c & 0xE0) == 0xC0 && bytes >= 2) {
        return ((unsigned int)(c & 0x1F) << 6) | (unsigned int)(s[1] & 0x3F);
    }
    if ((c & 0xF0) == 0xE0 && bytes >= 3) {
        return ((unsigned int)(c & 0x0F) << 12) | ((unsigned int)(s[1] & 0x3F) << 6) |
               (unsigned int)(s[2] & 0x3F);
    }
    if ((c & 0xF8) == 0xF0 && bytes >= 4) {
        return ((unsigned int)(c & 0x07) << 18) | ((unsigned int)(s[1] & 0x3F) << 12) |
               ((unsigned int)(s[2] & 0x3F) << 6) | (unsigned int)(s[3] & 0x3F);
    }
    return c;
}

static void xr_utf8_encode(unsigned int cp, char out[4], int *n) {
    if (cp < 0x80) {
        out[0] = (char)cp;
        *n = 1;
    } else if (cp < 0x800) {
        out[0] = (char)(0xC0 | (cp >> 6));
        out[1] = (char)(0x80 | (cp & 0x3F));
        *n = 2;
    } else if (cp < 0x10000) {
        out[0] = (char)(0xE0 | (cp >> 12));
        out[1] = (char)(0x80 | ((cp >> 6) & 0x3F));
        out[2] = (char)(0x80 | (cp & 0x3F));
        *n = 3;
    } else {
        out[0] = (char)(0xF0 | (cp >> 18));
        out[1] = (char)(0x80 | ((cp >> 12) & 0x3F));
        out[2] = (char)(0x80 | ((cp >> 6) & 0x3F));
        out[3] = (char)(0x80 | (cp & 0x3F));
        *n = 4;
    }
}

static long long xr_utf8_count(const char *data, long long bytes) {
    long long at = 0;
    long long count = 0;
    while (at < bytes) {
        long long step = xr_utf8_len((const unsigned char *)data + at);
        if (at + step > bytes) {
            step = 1;
        }
        at += step;
        count += 1;
    }
    return count;
}

// Byte offset of scalar `index` (clamped to `bytes`).
static long long xr_utf8_offset(const char *data, long long bytes, long long index) {
    long long at = 0;
    long long i = 0;
    while (i < index && at < bytes) {
        long long step = xr_utf8_len((const unsigned char *)data + at);
        if (at + step > bytes) {
            step = 1;
        }
        at += step;
        i += 1;
    }
    return at;
}

// ── constructors ─────────────────────────────────────────────────────────

static XVal xr_mk(int tag) {
    XVal v;
    v.tag = tag;
    v.i = 0;
    v.f = 0.0;
    v.p = NULL;
    return v;
}

XVal xr_nil(void) { return xr_mk(XT_NIL); }

XVal xr_int(long long n) {
    XVal v = xr_mk(XT_INT);
    v.i = n;
    return v;
}

XVal xr_float(double f) {
    XVal v = xr_mk(XT_FLOAT);
    v.f = f;
    return v;
}

XVal xr_bool(int b) {
    XVal v = xr_mk(XT_BOOL);
    v.i = b != 0 ? 1 : 0;
    return v;
}

XVal xr_str_n(const char *data, long long bytes) {
    XVal v = xr_mk(XT_STR);
    XStr *s = (XStr *)xr_alloc(sizeof(XStr));
    s->bytes = bytes;
    s->data = xr_copy_bytes(data, bytes);
    s->len = xr_utf8_count(s->data, s->bytes);
    v.p = s;
    return v;
}

XVal xr_str(const char *data) { return xr_str_n(data, (long long)strlen(data)); }

static XStr *xr_as_str(XVal v) { return (XStr *)v.p; }
static XList *xr_as_list(XVal v) { return (XList *)v.p; }
static XObj *xr_as_obj(XVal v) { return (XObj *)v.p; }

// ── errors ───────────────────────────────────────────────────────────────

static void xr_fail(const char *message) {
    fflush(stdout);
    fprintf(stderr, "%s\n", message);
    exit(1);
}

XVal xr_panic(XVal message) {
    XStr *s = xr_as_str(message);
    if (message.tag == XT_STR) {
        xr_fail(s->data);
    }
    xr_fail("panic");
    return xr_nil();
}

XVal xr_assert(XVal condition, XVal message) {
    if (xr_cond(condition)) {
        return xr_nil();
    }
    if (message.tag == XT_STR) {
        xr_fail(xr_as_str(message)->data);
    }
    xr_fail("assertion failed");
    return xr_nil();
}

int xr_cond(XVal v) {
    if (v.tag == XT_BOOL) {
        return v.i != 0;
    }
    if (v.tag == XT_NIL) {
        return 0;
    }
    return 1;
}

// ── strings ──────────────────────────────────────────────────────────────

XVal xr_str_concat(XVal a, XVal b) {
    if (a.tag != XT_STR || b.tag != XT_STR) {
        xr_fail("`+` expects two strings");
    }
    XStr *x = xr_as_str(a);
    XStr *y = xr_as_str(b);
    XVal v = xr_mk(XT_STR);
    XStr *s = (XStr *)xr_alloc(sizeof(XStr));
    s->bytes = x->bytes + y->bytes;
    s->data = (char *)xr_alloc((size_t)s->bytes + 1);
    memcpy(s->data, x->data, (size_t)x->bytes);
    memcpy(s->data + x->bytes, y->data, (size_t)y->bytes);
    s->data[s->bytes] = '\0';
    s->len = x->len + y->len;
    v.p = s;
    return v;
}

int xr_str_eq(XVal a, XVal b) {
    if (a.tag != XT_STR || b.tag != XT_STR) {
        return 0;
    }
    XStr *x = xr_as_str(a);
    XStr *y = xr_as_str(b);
    return x->bytes == y->bytes && memcmp(x->data, y->data, (size_t)x->bytes) == 0;
}

int xr_str_cmp(XVal a, XVal b) {
    XStr *x = xr_as_str(a);
    XStr *y = xr_as_str(b);
    long long n = x->bytes < y->bytes ? x->bytes : y->bytes;
    int c = memcmp(x->data, y->data, (size_t)n);
    if (c != 0) {
        return c < 0 ? -1 : 1;
    }
    if (x->bytes == y->bytes) {
        return 0;
    }
    return x->bytes < y->bytes ? -1 : 1;
}

XVal xr_str_slice(XVal s, long long start, long long end) {
    if (s.tag != XT_STR) {
        xr_fail("`.slice` expects a string receiver");
    }
    XStr *x = xr_as_str(s);
    if (start < 0) {
        start = 0;
    }
    if (end > x->len) {
        end = x->len;
    }
    if (end < start) {
        end = start;
    }
    long long from = xr_utf8_offset(x->data, x->bytes, start);
    long long to = xr_utf8_offset(x->data, x->bytes, end);
    return xr_str_n(x->data + from, to - from);
}

XVal xr_str_code_at(XVal s, long long index) {
    if (s.tag != XT_STR) {
        xr_fail("`.code_at` expects a string receiver");
    }
    XStr *x = xr_as_str(s);
    long long at = xr_utf8_offset(x->data, x->bytes, index);
    long long n = 0;
    if (at < x->bytes) {
        n = xr_utf8_len((const unsigned char *)x->data + at);
        if (at + n > x->bytes) {
            n = 1;
        }
    }
    return xr_int((long long)xr_utf8_decode((const unsigned char *)x->data + at, n));
}

int xr_str_starts_with(XVal s, XVal prefix) {
    XStr *x = xr_as_str(s);
    XStr *p = xr_as_str(prefix);
    return p->bytes <= x->bytes && memcmp(x->data, p->data, (size_t)p->bytes) == 0;
}

int xr_str_ends_with(XVal s, XVal suffix) {
    XStr *x = xr_as_str(s);
    XStr *p = xr_as_str(suffix);
    return p->bytes <= x->bytes &&
           memcmp(x->data + (x->bytes - p->bytes), p->data, (size_t)p->bytes) == 0;
}

int xr_str_contains(XVal s, XVal needle) {
    XStr *x = xr_as_str(s);
    XStr *n = xr_as_str(needle);
    if (n->bytes == 0) {
        return 1;
    }
    if (n->bytes > x->bytes) {
        return 0;
    }
    for (long long at = 0; at + n->bytes <= x->bytes; at += 1) {
        if (memcmp(x->data + at, n->data, (size_t)n->bytes) == 0) {
            return 1;
        }
    }
    return 0;
}

// ── lists ────────────────────────────────────────────────────────────────

XVal xr_list_new(void) {
    XVal v = xr_mk(XT_LIST);
    XList *l = (XList *)xr_alloc(sizeof(XList));
    l->len = 0;
    l->cap = 0;
    l->items = NULL;
    v.p = l;
    return v;
}

void xr_list_push(XVal list, XVal item) {
    if (list.tag != XT_LIST) {
        xr_fail("`.push` expects a list receiver");
    }
    XList *l = xr_as_list(list);
    if (l->len == l->cap) {
        long long cap = l->cap == 0 ? 8 : l->cap * 2;
        XVal *items = (XVal *)xr_alloc(sizeof(XVal) * (size_t)cap);
        if (l->len > 0) {
            memcpy(items, l->items, sizeof(XVal) * (size_t)l->len);
        }
        l->items = items;
        l->cap = cap;
    }
    l->items[l->len] = item;
    l->len += 1;
}

void xr_list_extend(XVal list, XVal other) {
    if (other.tag != XT_LIST) {
        xr_fail("list spread expects a list");
    }
    XList *l = xr_as_list(other);
    for (long long i = 0; i < l->len; i += 1) {
        xr_list_push(list, l->items[i]);
    }
}

XVal xr_list_lit(long long n, XVal *items) {
    XVal list = xr_list_new();
    for (long long i = 0; i < n; i += 1) {
        xr_list_push(list, items[i]);
    }
    return list;
}

XVal xr_range(XVal start, XVal end, long long inclusive) {
    long long from = start.i;
    long long to = end.i;
    if (inclusive) {
        to += 1;
    }
    XVal list = xr_list_new();
    for (long long i = from; i < to; i += 1) {
        xr_list_push(list, xr_int(i));
    }
    return list;
}

long long xr_len(XVal v) {
    if (v.tag == XT_STR) {
        return xr_as_str(v)->len;
    }
    if (v.tag == XT_LIST) {
        return xr_as_list(v)->len;
    }
    if (v.tag == XT_SB) {
        return ((XSb *)v.p)->len;
    }
    if (v.tag == XT_SET) {
        return ((XSet *)v.p)->len;
    }
    if (v.tag == XT_OBJ) {
        XObj *o = xr_as_obj(v);
        if (o->variant == NULL && strcmp(o->type, "Map") == 0) {
            return o->nfields;
        }
    }
    xr_fail("`.len` is not available on this value");
    return 0;
}

long long xr_byte_len(XVal v) {
    if (v.tag == XT_STR) {
        return xr_as_str(v)->bytes;
    }
    if (v.tag == XT_SB) {
        return ((XSb *)v.p)->bytes;
    }
    xr_fail("`.byte_len` is not available on this value");
    return 0;
}

XVal xr_index(XVal v, XVal index) {
    long long i = index.i;
    if (v.tag == XT_LIST) {
        XList *l = xr_as_list(v);
        if (i < 0 || i >= l->len) {
            xr_fail("list index out of bounds");
        }
        return l->items[i];
    }
    if (v.tag == XT_STR) {
        return xr_str_slice(v, i, i + 1);
    }
    if (v.tag == XT_SET) {
        XSet *s = (XSet *)v.p;
        if (i < 0 || i >= s->len) {
            xr_fail("set index out of bounds");
        }
        return s->items[i];
    }
    if (v.tag == XT_OBJ) {
        XObj *o = xr_as_obj(v);
        if (o->variant == NULL && strcmp(o->type, "Map") == 0) {
            if (i < 0 || i >= o->nfields) {
                xr_fail("map index out of bounds");
            }
            return xr_str(o->fields[i].name);
        }
    }
    xr_fail("this value is not indexable");
    return xr_nil();
}

void xr_set_index(XVal v, XVal index, XVal value) {
    long long i = index.i;
    if (v.tag == XT_LIST) {
        XList *l = xr_as_list(v);
        if (i < 0 || i >= l->len) {
            xr_fail("list index out of bounds");
        }
        l->items[i] = value;
        return;
    }
    xr_fail("this value does not support index assignment");
}

// ── objects, structs, enums ──────────────────────────────────────────────

XVal xr_struct(const char *type, long long n, const char **names, XVal *values) {
    XVal v = xr_mk(XT_OBJ);
    XObj *o = (XObj *)xr_alloc(sizeof(XObj));
    o->type = type;
    o->nfields = n;
    o->fields = (XField *)xr_alloc(sizeof(XField) * (size_t)(n == 0 ? 1 : n));
    for (long long i = 0; i < n; i += 1) {
        o->fields[i].name = names[i];
        o->fields[i].value = values[i];
    }
    o->variant = NULL;
    o->payload_len = 0;
    o->payload = NULL;
    o->debug = 0;
    v.p = o;
    return v;
}

// `#[derive(Debug)]`: the value renders as `Type(field: value, …)`.
XVal xr_struct_debug(const char *type, long long n, const char **names, XVal *values) {
    XVal v = xr_struct(type, n, names, values);
    ((XObj *)v.p)->debug = 1;
    return v;
}

int xr_is_struct(XVal v, const char *type) {
    if (v.tag != XT_OBJ) {
        return 0;
    }
    XObj *o = xr_as_obj(v);
    return o->variant == NULL && strcmp(o->type, type) == 0;
}

XVal xr_variant(const char *type, const char *variant, long long n, XVal *payload) {
    XVal v = xr_mk(XT_OBJ);
    XObj *o = (XObj *)xr_alloc(sizeof(XObj));
    o->type = type;
    o->nfields = 0;
    o->fields = NULL;
    o->variant = variant;
    o->payload_len = n;
    o->payload = NULL;
    o->debug = 0;
    if (n > 0) {
        o->payload = (XVal *)xr_alloc(sizeof(XVal) * (size_t)n);
        memcpy(o->payload, payload, sizeof(XVal) * (size_t)n);
    }
    v.p = o;
    return v;
}

// Object literals (`{ a: 1 }`) and `Map` values share the field-table shape.
// Members stay in insertion order; lookup is by name.
XVal xr_map_new(void) { return xr_struct("Map", 0, NULL, NULL); }

void xr_map_set(XVal map, XVal key, XVal value) {
    if (map.tag != XT_OBJ) {
        xr_fail("`.set` expects a map receiver");
    }
    XVal key_string = xr_key_string(key);
    XObj *o = xr_as_obj(map);
    const char *name = xr_as_str(key_string)->data;
    for (long long i = 0; i < o->nfields; i += 1) {
        if (strcmp(o->fields[i].name, name) == 0) {
            o->fields[i].value = value;
            return;
        }
    }
    long long count = o->nfields;
    XField *fields = (XField *)xr_alloc(sizeof(XField) * (size_t)(count + 1));
    for (long long i = 0; i < count; i += 1) {
        fields[i] = o->fields[i];
    }
    fields[count].name = name;
    fields[count].value = value;
    o->fields = fields;
    o->nfields = count + 1;
    o->type = "Map";
}

void xr_map_merge(XVal map, XVal other) {
    if (other.tag == XT_NIL) {
        return;
    }
    if (other.tag != XT_OBJ) {
        xr_fail("a map spread expects an object");
    }
    XObj *o = xr_as_obj(other);
    for (long long i = 0; i < o->nfields; i += 1) {
        xr_map_set(map, xr_str(o->fields[i].name), o->fields[i].value);
    }
}

static long long xr_map_find(XObj *o, const char *name) {
    for (long long i = 0; i < o->nfields; i += 1) {
        if (strcmp(o->fields[i].name, name) == 0) {
            return i;
        }
    }
    return -1;
}

static XVal xr_map_get(XVal map, XVal key) {
    if (map.tag != XT_OBJ) {
        xr_fail("`.get` expects a map receiver");
    }
    XVal key_string = xr_key_string(key);
    XObj *o = xr_as_obj(map);
    long long at = xr_map_find(o, xr_as_str(key_string)->data);
    return at < 0 ? xr_nil() : o->fields[at].value;
}

static int xr_map_has(XVal map, XVal key) {
    if (map.tag != XT_OBJ) {
        xr_fail("`.has` expects a map receiver");
    }
    XVal key_string = xr_key_string(key);
    return xr_map_find(xr_as_obj(map), xr_as_str(key_string)->data) >= 0;
}

static XVal xr_map_remove(XVal map, XVal key) {
    if (map.tag != XT_OBJ) {
        xr_fail("`.remove` expects a map receiver");
    }
    XVal key_string = xr_key_string(key);
    XObj *o = xr_as_obj(map);
    long long at = xr_map_find(o, xr_as_str(key_string)->data);
    if (at < 0) {
        return xr_nil();
    }
    XVal old = o->fields[at].value;
    for (long long i = at; i + 1 < o->nfields; i += 1) {
        o->fields[i] = o->fields[i + 1];
    }
    o->nfields -= 1;
    return old;
}

XVal xr_map_from_entries(XVal entries) {
    if (entries.tag != XT_LIST) {
        xr_fail("`Map::from_entries` expects a list of (key, value) pairs");
    }
    XVal map = xr_map_new();
    XList *list = xr_as_list(entries);
    for (long long i = 0; i < list->len; i += 1) {
        XVal entry = list->items[i];
        if (entry.tag != XT_LIST) {
            xr_fail("`Map::from_entries` expects each entry to be a (key, value) pair");
        }
        XList *pair = xr_as_list(entry);
        if (pair->len != 2) {
            xr_fail("`Map::from_entries` expects each entry to be a (key, value) pair");
        }
        xr_map_set(map, pair->items[0], pair->items[1]);
    }
    return map;
}

// ── sets ─────────────────────────────────────────────────────────────────

XVal xr_set_new(void) {
    XVal v = xr_mk(XT_SET);
    XSet *s = (XSet *)xr_alloc(sizeof(XSet));
    s->len = 0;
    s->cap = 0;
    s->items = NULL;
    v.p = s;
    return v;
}

static long long xr_set_find(XSet *s, XVal value) {
    for (long long i = 0; i < s->len; i += 1) {
        if (xr_eq(s->items[i], value)) {
            return i;
        }
    }
    return -1;
}

static void xr_set_push(XSet *s, XVal value) {
    if (s->len == s->cap) {
        long long cap = s->cap == 0 ? 8 : s->cap * 2;
        XVal *items = (XVal *)xr_alloc(sizeof(XVal) * (size_t)cap);
        if (s->len > 0) {
            memcpy(items, s->items, sizeof(XVal) * (size_t)s->len);
        }
        s->items = items;
        s->cap = cap;
    }
    s->items[s->len] = value;
    s->len += 1;
}

XVal xr_set_from_list(XVal list) {
    if (list.tag != XT_LIST) {
        xr_fail("`Set::from_list` expects a list");
    }
    XVal out = xr_set_new();
    XSet *set = (XSet *)out.p;
    XList *values = xr_as_list(list);
    for (long long i = 0; i < values->len; i += 1) {
        if (xr_set_find(set, values->items[i]) < 0) {
            xr_set_push(set, values->items[i]);
        }
    }
    return out;
}

static int xr_set_insert(XSet *set, XVal value) {
    if (xr_set_find(set, value) >= 0) {
        return 0;
    }
    xr_set_push(set, value);
    return 1;
}

static int xr_set_remove(XSet *set, XVal value) {
    long long at = xr_set_find(set, value);
    if (at < 0) {
        return 0;
    }
    for (long long i = at; i + 1 < set->len; i += 1) {
        set->items[i] = set->items[i + 1];
    }
    set->len -= 1;
    return 1;
}

XVal xr_ok(XVal value) { return xr_variant("Result", "Ok", 1, &value); }

XVal xr_error(const char *message) {
    const char *names[1];
    XVal values[1];
    names[0] = "message";
    values[0] = xr_str(message);
    return xr_struct("Error", 1, names, values);
}

XVal xr_err(const char *message) {
    XVal error = xr_error(message);
    return xr_variant("Result", "Err", 1, &error);
}

static void xr_qualified(const char *qualified, const char **type, const char **tag) {
    const char *sep = strstr(qualified, "::");
    if (sep == NULL) {
        *type = qualified;
        *tag = qualified;
        return;
    }
    static char left[256];
    long long n = (long long)(sep - qualified);
    if (n > 240) {
        n = 240;
    }
    memcpy(left, qualified, (size_t)n);
    left[n] = '\0';
    *type = left;
    *tag = sep + 2;
}

int xr_is_variant(XVal v, const char *qualified) {
    if (v.tag != XT_OBJ) {
        return 0;
    }
    XObj *o = xr_as_obj(v);
    if (o->variant == NULL) {
        return 0;
    }
    const char *type = NULL;
    const char *tag = NULL;
    xr_qualified(qualified, &type, &tag);
    return strcmp(o->type, type) == 0 && strcmp(o->variant, tag) == 0;
}

XVal xr_payload(XVal v, long long index) {
    if (v.tag != XT_OBJ) {
        xr_fail("this value has no payload");
    }
    XObj *o = xr_as_obj(v);
    if (index < 0 || index >= o->payload_len) {
        return xr_nil();
    }
    return o->payload[index];
}

XVal xr_get_field(XVal v, const char *name) {
    if (v.tag == XT_NIL) {
        xr_fail("cannot access a member of `null`");
    }
    if (v.tag == XT_OBJ) {
        XObj *o = xr_as_obj(v);
        for (long long i = 0; i < o->nfields; i += 1) {
            if (strcmp(o->fields[i].name, name) == 0) {
                return o->fields[i].value;
            }
        }
        // A brace map literal is the only object that answers `.size`
        // (spec `types/composite-types.md`); a real field named `size` wins
        // above.
        if (o->variant == NULL && strcmp(o->type, "Map") == 0 && strcmp(name, "size") == 0) {
            return xr_int(o->nfields);
        }
    }
    if (v.tag == XT_SET && strcmp(name, "size") == 0) {
        return xr_int(((XSet *)v.p)->len);
    }
    xr_fail("unknown member");
    return xr_nil();
}

XVal xr_get_field_opt(XVal v, const char *name) {
    if (v.tag == XT_NIL) {
        return xr_nil();
    }
    if (v.tag == XT_OBJ) {
        XObj *o = xr_as_obj(v);
        for (long long i = 0; i < o->nfields; i += 1) {
            if (strcmp(o->fields[i].name, name) == 0) {
                return o->fields[i].value;
            }
        }
        if (o->variant == NULL && strcmp(o->type, "Map") == 0 && strcmp(name, "size") == 0) {
            return xr_int(o->nfields);
        }
        return xr_nil();
    }
    if (v.tag == XT_SET && strcmp(name, "size") == 0) {
        return xr_int(((XSet *)v.p)->len);
    }
    return xr_nil();
}

void xr_set_field(XVal v, const char *name, XVal value) {
    if (v.tag != XT_OBJ) {
        xr_fail("this value does not support field assignment");
    }
    XObj *o = xr_as_obj(v);
    for (long long i = 0; i < o->nfields; i += 1) {
        if (strcmp(o->fields[i].name, name) == 0) {
            o->fields[i].value = value;
            return;
        }
    }
    XObj next;
    next.type = o->type;
    next.nfields = o->nfields + 1;
    next.fields = (XField *)xr_alloc(sizeof(XField) * (size_t)next.nfields);
    for (long long i = 0; i < o->nfields; i += 1) {
        next.fields[i] = o->fields[i];
    }
    next.fields[o->nfields].name = name;
    next.fields[o->nfields].value = value;
    next.variant = o->variant;
    next.payload_len = o->payload_len;
    next.payload = o->payload;
    *o = next;
}

