// Stage 1 bootstrap runtime.
//
// The stage 1 backend (`xuloc/src/backend.xulo`) lowers the generic AST to C, not
// to typed machine code: every expression has the same `XVal` shape and every
// user type is a field table. This file is the small runtime that C program
// links against. It is deliberately not the product runtime
// (`crates/xulo-runtime`); it exists so the bootstrap compiler can compile the
// bootstrap subset — including itself — on a host that has a C compiler.
//
// Values are leaked rather than collected. A bootstrap compiler runs once and
// exits, so this keeps the runtime small and predictable; it is not a memory
// model for user programs (those use the stage 0 runtime).
//
// Compile as part of the generated translation unit:
//     cc -O1 generated.c -o program
// `xuloc/runtime/runtime.c` has no `main` and no platform SDK dependency beyond libc.

// `popen`/`pclose` (and `lstat`) are POSIX, but strict `-std=c11` hides them
// behind a feature-test macro. Ask for the 2008 POSIX surface explicitly.
#if !defined(_WIN32) && !defined(_POSIX_C_SOURCE)
#define _POSIX_C_SOURCE 200809L
#endif

#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <errno.h>

#ifdef _WIN32
#include <direct.h>
#include <windows.h>
#define XR_POPEN _popen
#define XR_PCLOSE _pclose
#define XR_GETPID _getpid
#else
#include <dirent.h>
#include <fcntl.h>
#include <dirent.h>
#include <pwd.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>
#ifdef __APPLE__
#include <mach-o/dyld.h>
#endif
#define XR_POPEN popen
#define XR_PCLOSE pclose
#define XR_GETPID getpid
#endif

enum {
    XT_NIL = 0,
    XT_INT = 1,
    XT_FLOAT = 2,
    XT_BOOL = 3,
    XT_STR = 4,
    XT_LIST = 5,
    XT_OBJ = 6,
    XT_SB = 7
};

typedef struct XVal XVal;
typedef struct XStr XStr;
typedef struct XList XList;
typedef struct XField XField;
typedef struct XObj XObj;
typedef struct XSb XSb;

struct XVal {
    int tag;
    long long i;
    double f;
    void *p;
};

struct XStr {
    long long len;   // Unicode scalar values
    long long bytes; // UTF-8 bytes
    char *data;
};

struct XList {
    long long len;
    long long cap;
    XVal *items;
};

struct XField {
    const char *name;
    XVal value;
};

struct XObj {
    const char *type;
    long long nfields;
    XField *fields;
    const char *variant; // non-NULL for an enum value
    long long payload_len;
    XVal *payload;
};

struct XSb {
    long long len;
    long long bytes;
    char *data;
};

typedef XVal (*XrMethodFn)(long long argc, XVal *argv);

// Defined with the value predicates; declared here because the intrinsic
// helpers below use it.
int xr_cond(XVal v);

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
    v.p = o;
    return v;
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
    if (n > 0) {
        o->payload = (XVal *)xr_alloc(sizeof(XVal) * (size_t)n);
        memcpy(o->payload, payload, sizeof(XVal) * (size_t)n);
    }
    v.p = o;
    return v;
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
        return xr_nil();
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

// ── arithmetic and comparison ────────────────────────────────────────────

static int xr_is_num(XVal v) { return v.tag == XT_INT || v.tag == XT_FLOAT; }

static double xr_as_double(XVal v) { return v.tag == XT_INT ? (double)v.i : v.f; }

static long long xr_wrap_add(long long a, long long b) {
    return (long long)((unsigned long long)a + (unsigned long long)b);
}

static long long xr_wrap_sub(long long a, long long b) {
    return (long long)((unsigned long long)a - (unsigned long long)b);
}

static long long xr_wrap_mul(long long a, long long b) {
    return (long long)((unsigned long long)a * (unsigned long long)b);
}

XVal xr_add(XVal a, XVal b) {
    if (a.tag == XT_STR && b.tag == XT_STR) {
        return xr_str_concat(a, b);
    }
    if (a.tag == XT_LIST && b.tag == XT_LIST) {
        XVal out = xr_list_lit(0, NULL);
        xr_list_extend(out, a);
        xr_list_extend(out, b);
        return out;
    }
    if (a.tag == XT_INT && b.tag == XT_INT) {
        return xr_int(xr_wrap_add(a.i, b.i));
    }
    if (xr_is_num(a) && xr_is_num(b)) {
        return xr_float(xr_as_double(a) + xr_as_double(b));
    }
    xr_fail("`+` expects numbers, strings, or lists");
    return xr_nil();
}

XVal xr_sub(XVal a, XVal b) {
    if (a.tag == XT_INT && b.tag == XT_INT) {
        return xr_int(xr_wrap_sub(a.i, b.i));
    }
    if (xr_is_num(a) && xr_is_num(b)) {
        return xr_float(xr_as_double(a) - xr_as_double(b));
    }
    xr_fail("`-` expects numbers");
    return xr_nil();
}

XVal xr_mul(XVal a, XVal b) {
    if (a.tag == XT_INT && b.tag == XT_INT) {
        return xr_int(xr_wrap_mul(a.i, b.i));
    }
    if (xr_is_num(a) && xr_is_num(b)) {
        return xr_float(xr_as_double(a) * xr_as_double(b));
    }
    xr_fail("`*` expects numbers");
    return xr_nil();
}

XVal xr_div(XVal a, XVal b) {
    if (a.tag == XT_INT && b.tag == XT_INT) {
        if (b.i == 0) {
            xr_fail("division by zero");
        }
        return xr_int(a.i / b.i);
    }
    if (xr_is_num(a) && xr_is_num(b)) {
        return xr_float(xr_as_double(a) / xr_as_double(b));
    }
    xr_fail("`/` expects numbers");
    return xr_nil();
}

XVal xr_mod(XVal a, XVal b) {
    if (a.tag == XT_INT && b.tag == XT_INT) {
        if (b.i == 0) {
            xr_fail("division by zero");
        }
        return xr_int(a.i % b.i);
    }
    if (xr_is_num(a) && xr_is_num(b)) {
        return xr_float(fmod(xr_as_double(a), xr_as_double(b)));
    }
    xr_fail("`%` expects numbers");
    return xr_nil();
}

XVal xr_pow(XVal a, XVal b) {
    if (a.tag == XT_INT && b.tag == XT_INT && b.i >= 0) {
        long long result = 1;
        long long base = a.i;
        long long n = b.i;
        while (n > 0) {
            if ((n & 1) == 1) {
                result = xr_wrap_mul(result, base);
            }
            base = xr_wrap_mul(base, base);
            n >>= 1;
        }
        return xr_int(result);
    }
    return xr_float(pow(xr_as_double(a), xr_as_double(b)));
}

XVal xr_neg(XVal a) {
    if (a.tag == XT_INT) {
        return xr_int(xr_wrap_sub(0, a.i));
    }
    if (a.tag == XT_FLOAT) {
        return xr_float(-a.f);
    }
    xr_fail("unary `-` expects a number");
    return xr_nil();
}

XVal xr_not(XVal a) { return xr_bool(!xr_cond(a)); }

XVal xr_bitnot(XVal a) {
    if (a.tag != XT_INT) {
        xr_fail("unary `~` expects an integer");
    }
    return xr_int(~a.i);
}

XVal xr_band(XVal a, XVal b) {
    if (a.tag != XT_INT || b.tag != XT_INT) {
        xr_fail("`&` expects integers");
    }
    return xr_int(a.i & b.i);
}

XVal xr_bor(XVal a, XVal b) {
    if (a.tag != XT_INT || b.tag != XT_INT) {
        xr_fail("`|` expects integers");
    }
    return xr_int(a.i | b.i);
}

XVal xr_bxor(XVal a, XVal b) {
    if (a.tag != XT_INT || b.tag != XT_INT) {
        xr_fail("`^` expects integers");
    }
    return xr_int(a.i ^ b.i);
}

XVal xr_shl(XVal a, XVal b) {
    if (a.tag != XT_INT || b.tag != XT_INT) {
        xr_fail("`<<` expects integers");
    }
    long long n = b.i;
    if (n < 0 || n >= 64) {
        return xr_int(0);
    }
    return xr_int((long long)((unsigned long long)a.i << (unsigned long long)n));
}

XVal xr_shr(XVal a, XVal b) {
    if (a.tag != XT_INT || b.tag != XT_INT) {
        xr_fail("`>>` expects integers");
    }
    long long n = b.i;
    if (n < 0) {
        n = 0;
    }
    if (n >= 64) {
        return xr_int(a.i < 0 ? -1 : 0);
    }
    return xr_int(a.i >> n);
}

int xr_eq(XVal a, XVal b) {
    if (xr_is_num(a) && xr_is_num(b)) {
        if (a.tag == XT_INT && b.tag == XT_INT) {
            return a.i == b.i;
        }
        return xr_as_double(a) == xr_as_double(b);
    }
    if (a.tag != b.tag) {
        return 0;
    }
    switch (a.tag) {
    case XT_NIL:
        return 1;
    case XT_BOOL:
        return a.i == b.i;
    case XT_STR:
        return xr_str_eq(a, b);
    case XT_LIST: {
        XList *x = xr_as_list(a);
        XList *y = xr_as_list(b);
        if (x->len != y->len) {
            return 0;
        }
        for (long long i = 0; i < x->len; i += 1) {
            if (!xr_eq(x->items[i], y->items[i])) {
                return 0;
            }
        }
        return 1;
    }
    case XT_SB: {
        XSb *x = (XSb *)a.p;
        XSb *y = (XSb *)b.p;
        return x == y || (x->bytes == y->bytes && memcmp(x->data, y->data, (size_t)x->bytes) == 0);
    }
    case XT_OBJ: {
        XObj *x = xr_as_obj(a);
        XObj *y = xr_as_obj(b);
        if (x == y) {
            return 1;
        }
        if (strcmp(x->type, y->type) != 0) {
            return 0;
        }
        if ((x->variant == NULL) != (y->variant == NULL)) {
            return 0;
        }
        if (x->variant != NULL && strcmp(x->variant, y->variant) != 0) {
            return 0;
        }
        if (x->nfields != y->nfields || x->payload_len != y->payload_len) {
            return 0;
        }
        for (long long i = 0; i < x->nfields; i += 1) {
            XVal other = xr_get_field_opt(b, x->fields[i].name);
            if (!xr_eq(x->fields[i].value, other)) {
                return 0;
            }
        }
        for (long long i = 0; i < x->payload_len; i += 1) {
            if (!xr_eq(x->payload[i], y->payload[i])) {
                return 0;
            }
        }
        return 1;
    }
    default:
        return 0;
    }
}

static int xr_order(XVal a, XVal b) {
    if (xr_is_num(a) && xr_is_num(b)) {
        double x = xr_as_double(a);
        double y = xr_as_double(b);
        if (x < y) {
            return -1;
        }
        if (x > y) {
            return 1;
        }
        return 0;
    }
    if (a.tag == XT_STR && b.tag == XT_STR) {
        return xr_str_cmp(a, b);
    }
    xr_fail("comparison expects two numbers or two strings");
    return 0;
}

XVal xr_lt(XVal a, XVal b) { return xr_bool(xr_order(a, b) < 0); }
XVal xr_le(XVal a, XVal b) { return xr_bool(xr_order(a, b) <= 0); }
XVal xr_gt(XVal a, XVal b) { return xr_bool(xr_order(a, b) > 0); }
XVal xr_ge(XVal a, XVal b) { return xr_bool(xr_order(a, b) >= 0); }

XVal xr_nullish(XVal a, XVal b) { return a.tag == XT_NIL ? b : a; }

int xr_is(XVal v, const char *type) {
    char name[128];
    long long n = 0;
    while (type[n] != '\0' && type[n] != '<' && n < 127) {
        name[n] = type[n];
        n += 1;
    }
    name[n] = '\0';
    if (strcmp(name, "number") == 0 || strcmp(name, "int") == 0 || strcmp(name, "u64") == 0 ||
        strcmp(name, "usize") == 0 || strcmp(name, "isize") == 0) {
        return v.tag == XT_INT;
    }
    if (strcmp(name, "float") == 0 || strcmp(name, "f64") == 0) {
        return v.tag == XT_FLOAT;
    }
    if (strcmp(name, "string") == 0) {
        return v.tag == XT_STR;
    }
    if (strcmp(name, "bool") == 0) {
        return v.tag == XT_BOOL;
    }
    if (strcmp(name, "null") == 0) {
        return v.tag == XT_NIL;
    }
    if (strcmp(name, "unknown") == 0) {
        return 1;
    }
    if (strcmp(name, "List") == 0) {
        return v.tag == XT_LIST;
    }
    if (v.tag == XT_OBJ) {
        return strcmp(xr_as_obj(v)->type, name) == 0;
    }
    return 0;
}

// ── formatting ───────────────────────────────────────────────────────────

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
            return xr_str(buffer);
        }
    }
    snprintf(buffer, sizeof(buffer), "%.17g", f);
    return xr_str(buffer);
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
    case XT_OBJ: {
        XObj *o = xr_as_obj(v);
        xr_append_str(buffer, len, cap, o->type, (long long)strlen(o->type));
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
        XrMethodFn fn = xr_lookup_method(o->type, name);
        if (fn != NULL) {
            return fn(n + 1, xr_args_with_self(recv, n, args));
        }
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
    if (strcmp(name, "split") == 0 && recv.tag == XT_STR) {
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
    (void)n;
    (void)args;
    if (strcmp(type, "StringBuilder") == 0 && strcmp(name, "new") == 0) {
        return xr_sb_new();
    }
    xr_fail("unknown static method");
    return xr_nil();
}

// ── std::fs / std::io / std::os / std::process ───────────────────────────

static char *xr_read_stream(FILE *stream, long long *bytes) {
    long long cap = 4096;
    long long len = 0;
    char *data = (char *)xr_alloc((size_t)cap);
    size_t n = 0;
    while ((n = fread(data + len, 1, (size_t)(cap - len - 1), stream)) > 0) {
        len += (long long)n;
        if (len + 1 >= cap) {
            long long next_cap = cap * 2;
            char *next = (char *)xr_alloc((size_t)next_cap);
            memcpy(next, data, (size_t)len);
            data = next;
            cap = next_cap;
        }
    }
    data[len] = '\0';
    *bytes = len;
    return data;
}

// ── filesystem walk ──────────────────────────────────────────────────────

typedef struct {
    char **items;
    long long len;
    long long cap;
} XrNames;

static void xr_names_push(XrNames *names, char *name) {
    if (names->len == names->cap) {
        long long cap = names->cap == 0 ? 16 : names->cap * 2;
        char **next = (char **)xr_alloc((size_t)cap * sizeof(char *));
        if (names->len > 0) {
            memcpy(next, names->items, (size_t)names->len * sizeof(char *));
        }
        names->items = next;
        names->cap = cap;
    }
    names->items[names->len] = name;
    names->len += 1;
}

static int xr_name_cmp(const void *a, const void *b) {
    return strcmp(*(char *const *)a, *(char *const *)b);
}

static char *xr_path_join(const char *dir, const char *name) {
    size_t dir_len = strlen(dir);
    size_t name_len = strlen(name);
    int slash = dir_len > 0 && dir[dir_len - 1] != '/' && dir[dir_len - 1] != '\\';
    char *out = (char *)xr_alloc(dir_len + name_len + (slash ? 2 : 1));
    memcpy(out, dir, dir_len);
    if (slash) {
        out[dir_len] = '/';
        dir_len += 1;
    }
    memcpy(out + dir_len, name, name_len + 1);
    return out;
}

// A symlink is listed but never descended into: a link cycle cannot loop and a
// link out of the tree cannot widen the walk (same contract as `std::fs::walk`).
static int xr_is_directory(const char *path) {
#ifdef _WIN32
    DWORD attrs = GetFileAttributesA(path);
    if (attrs == INVALID_FILE_ATTRIBUTES) {
        return 0;
    }
    if ((attrs & FILE_ATTRIBUTE_REPARSE_POINT) != 0) {
        return 0;
    }
    return (attrs & FILE_ATTRIBUTE_DIRECTORY) != 0;
#else
    struct stat st;
    if (lstat(path, &st) != 0) {
        return 0;
    }
    return S_ISDIR(st.st_mode) && !S_ISLNK(st.st_mode);
#endif
}

// Names in one directory, sorted; the caller frees nothing (values leak).
static int xr_read_names(const char *dir, XrNames *names, char *error, size_t error_size) {
#ifdef _WIN32
    char *pattern = xr_path_join(dir, "*");
    WIN32_FIND_DATAA data;
    HANDLE handle = FindFirstFileA(pattern, &data);
    if (handle == INVALID_HANDLE_VALUE) {
        snprintf(error, error_size, "cannot walk `%s`", dir);
        return 0;
    }
    do {
        if (strcmp(data.cFileName, ".") == 0 || strcmp(data.cFileName, "..") == 0) {
            continue;
        }
        xr_names_push(names, xr_copy_bytes(data.cFileName, (long long)strlen(data.cFileName)));
    } while (FindNextFileA(handle, &data) != 0);
    FindClose(handle);
#else
    DIR *stream = opendir(dir);
    if (stream == NULL) {
        snprintf(error, error_size, "cannot walk `%s`", dir);
        return 0;
    }
    struct dirent *entry;
    while ((entry = readdir(stream)) != NULL) {
        if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) {
            continue;
        }
        xr_names_push(names, xr_copy_bytes(entry->d_name, (long long)strlen(entry->d_name)));
    }
    closedir(stream);
#endif
    if (names->len > 1) {
        qsort(names->items, (size_t)names->len, sizeof(char *), xr_name_cmp);
    }
    return 1;
}

static int xr_walk_into(const char *dir, XVal out, char *error, size_t error_size) {
    XrNames names = {NULL, 0, 0};
    if (!xr_read_names(dir, &names, error, error_size)) {
        return 0;
    }
    char **dirs = (char **)xr_alloc((size_t)(names.len == 0 ? 1 : names.len) * sizeof(char *));
    long long dir_count = 0;
    for (long long i = 0; i < names.len; i += 1) {
        char *child = xr_path_join(dir, names.items[i]);
        xr_list_push(out, xr_str(child));
        if (xr_is_directory(child)) {
            dirs[dir_count] = child;
            dir_count += 1;
        }
    }
    for (long long i = 0; i < dir_count; i += 1) {
        if (!xr_walk_into(dirs[i], out, error, error_size)) {
            return 0;
        }
    }
    return 1;
}

XVal xr_fs_walk(XVal path) {
    if (path.tag != XT_STR) {
        return xr_err("std::fs::walk expects a string path");
    }
    XStr *p = xr_as_str(path);
    if (!xr_is_directory(p->data)) {
        char message[512];
        snprintf(message, sizeof(message), "cannot walk `%s`", p->data);
        return xr_err(message);
    }
    XVal out = xr_list_new();
    xr_list_push(out, xr_str_n(p->data, p->bytes));
    char error[512];
    error[0] = '\0';
    if (!xr_walk_into(p->data, out, error, sizeof(error))) {
        return xr_err(error[0] == '\0' ? "cannot walk the directory" : error);
    }
    return xr_ok(out);
}

XVal xr_fs_read(XVal path) {
    if (path.tag != XT_STR) {
        return xr_err("std::fs::read expects a string path");
    }
    XStr *p = xr_as_str(path);
    FILE *file = fopen(p->data, "rb");
    if (file == NULL) {
        char message[512];
        snprintf(message, sizeof(message), "cannot read `%s`", p->data);
        return xr_err(message);
    }
    long long bytes = 0;
    char *data = xr_read_stream(file, &bytes);
    fclose(file);
    return xr_ok(xr_str_n(data, bytes));
}

XVal xr_fs_write(XVal path, XVal text) {
    if (path.tag != XT_STR || text.tag != XT_STR) {
        return xr_err("std::fs::write expects a string path and text");
    }
    XStr *p = xr_as_str(path);
    XStr *s = xr_as_str(text);
    FILE *file = fopen(p->data, "wb");
    if (file == NULL) {
        char message[512];
        snprintf(message, sizeof(message), "cannot write `%s`", p->data);
        return xr_err(message);
    }
    if (s->bytes > 0) {
        fwrite(s->data, 1, (size_t)s->bytes, file);
    }
    fclose(file);
    return xr_ok(xr_nil());
}

XVal xr_fs_mkdir(XVal path) {
    if (path.tag != XT_STR) {
        return xr_err("std::fs::mkdir expects a string path");
    }
    XStr *p = xr_as_str(path);
#ifdef _WIN32
    int rc = _mkdir(p->data);
#else
    int rc = mkdir(p->data, 0777);
#endif
    if (rc != 0 && errno != EEXIST) {
        char message[512];
        snprintf(message, sizeof(message), "cannot create `%s`", p->data);
        return xr_err(message);
    }
    return xr_ok(xr_nil());
}

#ifndef _WIN32
static void xr_walk_collect(XVal out, const char *path) {
    DIR *dir = opendir(path);
    if (dir == NULL) {
        return;
    }
    struct dirent *entry;
    while ((entry = readdir(dir)) != NULL) {
        const char *name = entry->d_name;
        if (strcmp(name, ".") == 0 || strcmp(name, "..") == 0) {
            continue;
        }
        size_t size = strlen(path) + strlen(name) + 2;
        char *child = (char *)xr_alloc(size);
        snprintf(child, size, "%s/%s", path, name);
        struct stat info;
        int is_dir = stat(child, &info) == 0 && S_ISDIR(info.st_mode);
        if (is_dir) {
            xr_walk_collect(out, child);
        } else {
            xr_list_push(out, xr_str(child));
        }
    }
    closedir(dir);
}
#endif

XVal xr_fs_walk(XVal dir) {
    if (dir.tag != XT_STR) {
        return xr_err("std::fs::walk expects a string path");
    }
#ifdef _WIN32
    (void)dir;
    return xr_err("std::fs::walk is not supported on this host");
#else
    XVal out = xr_list_new();
    xr_walk_collect(out, xr_as_str(dir)->data);
    return xr_ok(out);
#endif
}

// ── std::path (POSIX string semantics; never touches the host) ──────────

typedef struct {
    const char *data;
    long long len;
} XrPathSlice;

static long long xr_path_trim_slashes(const char *data, long long n) {
    while (n > 0 && data[n - 1] == '/') {
        n -= 1;
    }
    return n;
}

static long long xr_path_base_start(const char *data, long long n) {
    long long at = n;
    while (at > 0 && data[at - 1] != '/') {
        at -= 1;
    }
    return at;
}

static long long xr_path_ext_start(const char *data, long long start, long long n) {
    // The dot that begins the extension, or -1 for none. A leading dot is a
    // hidden-file marker and a trailing dot has no extension text.
    long long dot = -1;
    for (long long i = start; i < n; i += 1) {
        if (data[i] == '.') {
            dot = i;
        }
    }
    if (dot <= start || dot == n - 1) {
        return -1;
    }
    return dot;
}

static long long xr_path_components(const char *data, long long n, XrPathSlice *out) {
    long long count = 0;
    long long i = 0;
    while (i < n) {
        while (i < n && data[i] == '/') {
            i += 1;
        }
        long long start = i;
        while (i < n && data[i] != '/') {
            i += 1;
        }
        long long len = i - start;
        if (len == 0) {
            continue;
        }
        if (len == 1 && data[start] == '.') {
            continue;
        }
        out[count].data = data + start;
        out[count].len = len;
        count += 1;
    }
    return count;
}

static int xr_path_is_dotdot(XrPathSlice slice) {
    return slice.len == 2 && slice.data[0] == '.' && slice.data[1] == '.';
}

XVal xr_path_join(XVal a, XVal b) {
    if (a.tag != XT_STR || b.tag != XT_STR) {
        xr_fail("std::path::join expects two strings");
    }
    XStr *sa = xr_as_str(a);
    XStr *sb = xr_as_str(b);
    if (sb->bytes > 0 && sb->data[0] == '/') {
        return b;
    }
    if (sa->bytes == 0) {
        return b;
    }
    long long size = sa->bytes + sb->bytes + 2;
    char *buffer = (char *)xr_alloc((size_t)size);
    memcpy(buffer, sa->data, (size_t)sa->bytes);
    long long at = sa->bytes;
    if (sa->data[sa->bytes - 1] != '/') {
        buffer[at] = '/';
        at += 1;
    }
    memcpy(buffer + at, sb->data, (size_t)sb->bytes);
    at += sb->bytes;
    return xr_str_n(buffer, at);
}

XVal xr_path_is_absolute(XVal path) {
    if (path.tag != XT_STR) {
        xr_fail("std::path::is_absolute expects a string");
    }
    XStr *s = xr_as_str(path);
    return xr_bool(s->bytes > 0 && s->data[0] == '/');
}

XVal xr_path_basename(XVal path) {
    if (path.tag != XT_STR) {
        xr_fail("std::path::basename expects a string");
    }
    XStr *s = xr_as_str(path);
    long long n = xr_path_trim_slashes(s->data, s->bytes);
    long long start = xr_path_base_start(s->data, n);
    return xr_str_n(s->data + start, n - start);
}

XVal xr_path_dirname(XVal path) {
    if (path.tag != XT_STR) {
        xr_fail("std::path::dirname expects a string");
    }
    XStr *s = xr_as_str(path);
    int absolute = s->bytes > 0 && s->data[0] == '/';
    long long n = xr_path_trim_slashes(s->data, s->bytes);
    if (n == 0) {
        return xr_str(absolute ? "/" : "");
    }
    long long start = xr_path_base_start(s->data, n);
    if (start == 0) {
        return xr_str("");
    }
    long long end = start - 1;
    while (end > 0 && s->data[end - 1] == '/') {
        end -= 1;
    }
    if (end == 0) {
        return xr_str("/");
    }
    return xr_str_n(s->data, end);
}

XVal xr_path_ext(XVal path) {
    if (path.tag != XT_STR) {
        xr_fail("std::path::ext expects a string");
    }
    XStr *s = xr_as_str(path);
    long long n = xr_path_trim_slashes(s->data, s->bytes);
    long long start = xr_path_base_start(s->data, n);
    long long dot = xr_path_ext_start(s->data, start, n);
    if (dot < 0) {
        return xr_str("");
    }
    return xr_str_n(s->data + dot, n - dot);
}

XVal xr_path_stem(XVal path) {
    XVal base = xr_path_basename(path);
    XVal ext = xr_path_ext(path);
    XStr *sb = xr_as_str(base);
    XStr *se = xr_as_str(ext);
    long long len = sb->bytes - se->bytes;
    if (len < 0) {
        len = 0;
    }
    return xr_str_n(sb->data, len);
}

XVal xr_path_with_ext(XVal path, XVal ext) {
    if (path.tag != XT_STR || ext.tag != XT_STR) {
        xr_fail("std::path::with_ext expects two strings");
    }
    XStr *s = xr_as_str(path);
    XStr *e = xr_as_str(ext);
    const char *ext_data = e->data;
    long long ext_len = e->bytes;
    if (ext_len > 0 && ext_data[0] == '.') {
        ext_data += 1;
        ext_len -= 1;
    }
    long long n = xr_path_trim_slashes(s->data, s->bytes);
    long long start = xr_path_base_start(s->data, n);
    long long dot = xr_path_ext_start(s->data, start, n);
    long long stem_end = dot < 0 ? n : dot;
    XVal dir = xr_path_dirname(path);
    XStr *sd = xr_as_str(dir);
    long long size = sd->bytes + (stem_end - start) + ext_len + 4;
    char *buffer = (char *)xr_alloc((size_t)size);
    long long at = 0;
    if (sd->bytes > 0) {
        memcpy(buffer, sd->data, (size_t)sd->bytes);
        at = sd->bytes;
        if (sd->data[sd->bytes - 1] != '/') {
            buffer[at] = '/';
            at += 1;
        }
    }
    memcpy(buffer + at, s->data + start, (size_t)(stem_end - start));
    at += stem_end - start;
    if (ext_len > 0) {
        buffer[at] = '.';
        at += 1;
        memcpy(buffer + at, ext_data, (size_t)ext_len);
        at += ext_len;
    }
    return xr_str_n(buffer, at);
}

XVal xr_path_normalize(XVal path) {
    if (path.tag != XT_STR) {
        xr_fail("std::path::normalize expects a string");
    }
    XStr *s = xr_as_str(path);
    int absolute = s->bytes > 0 && s->data[0] == '/';
    long long cap = s->bytes + 1;
    XrPathSlice *stack = (XrPathSlice *)xr_alloc(sizeof(XrPathSlice) * (size_t)cap);
    long long count = 0;
    long long i = 0;
    while (i < s->bytes) {
        while (i < s->bytes && s->data[i] == '/') {
            i += 1;
        }
        long long start = i;
        while (i < s->bytes && s->data[i] != '/') {
            i += 1;
        }
        long long len = i - start;
        if (len == 0) {
            continue;
        }
        XrPathSlice part;
        part.data = s->data + start;
        part.len = len;
        if (len == 1 && part.data[0] == '.') {
            continue;
        }
        if (xr_path_is_dotdot(part)) {
            if (count > 0 && !xr_path_is_dotdot(stack[count - 1])) {
                count -= 1;
                continue;
            }
            if (absolute) {
                continue;
            }
            stack[count] = part;
            count += 1;
            continue;
        }
        stack[count] = part;
        count += 1;
    }
    if (count == 0) {
        return xr_str(absolute ? "/" : ".");
    }
    long long size = 1;
    for (long long k = 0; k < count; k += 1) {
        size += stack[k].len + 1;
    }
    char *buffer = (char *)xr_alloc((size_t)size);
    long long at = 0;
    if (absolute) {
        buffer[at] = '/';
        at += 1;
    }
    for (long long k = 0; k < count; k += 1) {
        if (k > 0) {
            buffer[at] = '/';
            at += 1;
        }
        memcpy(buffer + at, stack[k].data, (size_t)stack[k].len);
        at += stack[k].len;
    }
    return xr_str_n(buffer, at);
}

XVal xr_path_split(XVal path) {
    if (path.tag != XT_STR) {
        xr_fail("std::path::split expects a string");
    }
    const char *names[3] = {"dir", "base", "ext"};
    XVal values[3];
    values[0] = xr_path_dirname(path);
    values[1] = xr_path_basename(path);
    values[2] = xr_path_ext(path);
    return xr_struct("PathSplit", 3, names, values);
}

XVal xr_path_relative(XVal from, XVal to) {
    if (from.tag != XT_STR || to.tag != XT_STR) {
        xr_fail("std::path::relative expects two strings");
    }
    XVal nf = xr_path_normalize(from);
    XVal nt = xr_path_normalize(to);
    XStr *sf = xr_as_str(nf);
    XStr *st = xr_as_str(nt);
    int absolute = sf->bytes > 0 && sf->data[0] == '/';
    int target_absolute = st->bytes > 0 && st->data[0] == '/';
    if (absolute != target_absolute) {
        return xr_nil();
    }
    XrPathSlice *fc = (XrPathSlice *)xr_alloc(sizeof(XrPathSlice) * (size_t)(sf->bytes + 1));
    XrPathSlice *tc = (XrPathSlice *)xr_alloc(sizeof(XrPathSlice) * (size_t)(st->bytes + 1));
    long long fn = xr_path_components(sf->data, sf->bytes, fc);
    long long tn = xr_path_components(st->data, st->bytes, tc);
    long long common = 0;
    while (common < fn && common < tn && fc[common].len == tc[common].len
           && memcmp(fc[common].data, tc[common].data, (size_t)fc[common].len) == 0) {
        common += 1;
    }
    if (!absolute) {
        // Climbing above the shared anchor needs a name neither path carries.
        for (long long k = common; k < fn; k += 1) {
            if (xr_path_is_dotdot(fc[k])) {
                return xr_nil();
            }
        }
    }
    long long size = 2;
    for (long long k = common; k < fn; k += 1) {
        size += 3;
    }
    for (long long k = common; k < tn; k += 1) {
        size += tc[k].len + 1;
    }
    char *buffer = (char *)xr_alloc((size_t)size);
    long long at = 0;
    for (long long k = common; k < fn; k += 1) {
        memcpy(buffer + at, "../", 3);
        at += 3;
    }
    for (long long k = common; k < tn; k += 1) {
        if (at > 0 && buffer[at - 1] != '/') {
            buffer[at] = '/';
            at += 1;
        }
        memcpy(buffer + at, tc[k].data, (size_t)tc[k].len);
        at += tc[k].len;
    }
    if (at == 0) {
        return xr_str(absolute ? "/" : ".");
    }
    return xr_str_n(buffer, at);
}

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

XVal xr_process_run(XVal command) {    if (command.tag != XT_STR) {
        return xr_err("std::process::run expects a command string");
    }
    XStr *cmd = xr_as_str(command);
    char *full = (char *)xr_alloc((size_t)cmd->bytes + 6);
    if (cmd->bytes > 0) {
        memcpy(full, cmd->data, (size_t)cmd->bytes);
    }
    memcpy(full + cmd->bytes, " 2>&1", 5);
    FILE *pipe = XR_POPEN(full, "r");
    if (pipe == NULL) {
        return xr_err("cannot start the command");
    }
    long long bytes = 0;
    char *data = xr_read_stream(pipe, &bytes);
    int status = XR_PCLOSE(pipe);
    const char *names[3];
    XVal values[3];
    names[0] = "stdout";
    names[1] = "stderr";
    names[2] = "code";
    values[0] = xr_str_n(data, bytes);
    values[1] = xr_str("");
    values[2] = xr_int(status);
    return xr_ok(xr_struct("RunResult", 3, names, values));
}

// ── std::process::exec ───────────────────────────────────────────────────
//
// Unlike `xr_process_run` (which goes through the host shell), `exec` takes the
// program and its arguments separately, so paths with spaces never need shell
// quoting. stdout and stderr are captured into one pipe.

static XVal xr_run_result(const char *data, long long bytes, long long code) {
    const char *names[3];
    XVal values[3];
    names[0] = "stdout";
    names[1] = "stderr";
    names[2] = "code";
    values[0] = xr_str_n(data == NULL ? "" : data, bytes);
    values[1] = xr_str("");
    values[2] = xr_int(code);
    return xr_ok(xr_struct("RunResult", 3, names, values));
}

#ifdef _WIN32

static void xr_cmd_append_arg(char **buf, long long *len, long long *cap, const char *text) {
    long long need = (long long)strlen(text) * 2 + 4;
    if (*len + need > *cap) {
        long long next_cap = *cap == 0 ? 256 : *cap;
        while (next_cap < *len + need) {
            next_cap *= 2;
        }
        char *next = (char *)xr_alloc((size_t)next_cap);
        if (*len > 0) {
            memcpy(next, *buf, (size_t)*len);
        }
        *buf = next;
        *cap = next_cap;
    }
    char *out = *buf;
    out[(*len)++] = '"';
    for (const char *p = text; *p != '\0'; p += 1) {
        if (*p == '"') {
            out[(*len)++] = '\\';
        }
        out[(*len)++] = *p;
    }
    out[(*len)++] = '"';
    out[*len] = '\0';
}

XVal xr_process_exec(XVal program, XVal args) {
    if (program.tag != XT_STR || args.tag != XT_LIST) {
        return xr_err("std::process::exec expects a program string and an argument list");
    }
    XStr *exe = xr_as_str(program);
    char *cmdline = NULL;
    long long len = 0;
    long long cap = 0;
    xr_cmd_append_arg(&cmdline, &len, &cap, exe->data);
    XList *list = xr_as_list(args);
    for (long long i = 0; i < list->len; i += 1) {
        if (list->items[i].tag != XT_STR) {
            return xr_err("std::process::exec arguments must be strings");
        }
        cmdline[len++] = ' ';
        cmdline[len] = '\0';
        xr_cmd_append_arg(&cmdline, &len, &cap, xr_as_str(list->items[i])->data);
    }

    SECURITY_ATTRIBUTES sa;
    sa.nLength = sizeof(sa);
    sa.lpSecurityDescriptor = NULL;
    sa.bInheritHandle = TRUE;
    HANDLE read_pipe = NULL;
    HANDLE write_pipe = NULL;
    if (!CreatePipe(&read_pipe, &write_pipe, &sa, 0)) {
        return xr_err("cannot create a pipe for std::process::exec");
    }
    SetHandleInformation(read_pipe, HANDLE_FLAG_INHERIT, 0);

    STARTUPINFOA si;
    ZeroMemory(&si, sizeof(si));
    si.cb = sizeof(si);
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdInput = NULL;
    si.hStdOutput = write_pipe;
    si.hStdError = write_pipe;

    PROCESS_INFORMATION pi;
    ZeroMemory(&pi, sizeof(pi));
    BOOL started = CreateProcessA(NULL, cmdline, NULL, NULL, TRUE, CREATE_NO_WINDOW, NULL, NULL, &si, &pi);
    CloseHandle(write_pipe);
    if (!started) {
        CloseHandle(read_pipe);
        return xr_err("cannot start std::process::exec");
    }

    long long bytes = 0;
    long long buffer_cap = 4096;
    char *data = (char *)xr_alloc((size_t)buffer_cap);
    DWORD got = 0;
    while (ReadFile(read_pipe, data + bytes, (DWORD)(buffer_cap - bytes - 1), &got, NULL) && got > 0) {
        bytes += (long long)got;
        if (bytes + 1 >= buffer_cap) {
            long long next_cap = buffer_cap * 2;
            char *next = (char *)xr_alloc((size_t)next_cap);
            memcpy(next, data, (size_t)bytes);
            data = next;
            buffer_cap = next_cap;
        }
    }
    data[bytes] = '\0';
    CloseHandle(read_pipe);
    WaitForSingleObject(pi.hProcess, INFINITE);
    DWORD status = 70;
    GetExitCodeProcess(pi.hProcess, &status);
    CloseHandle(pi.hProcess);
    CloseHandle(pi.hThread);
    return xr_run_result(data, bytes, (long long)status);
}

#else

XVal xr_process_exec(XVal program, XVal args) {
    if (program.tag != XT_STR || args.tag != XT_LIST) {
        return xr_err("std::process::exec expects a program string and an argument list");
    }
    XStr *exe = xr_as_str(program);
    XList *list = xr_as_list(args);
    char **argv = (char **)xr_alloc(sizeof(char *) * (size_t)(list->len + 2));
    argv[0] = exe->data;
    for (long long i = 0; i < list->len; i += 1) {
        if (list->items[i].tag != XT_STR) {
            return xr_err("std::process::exec arguments must be strings");
        }
        argv[i + 1] = xr_as_str(list->items[i])->data;
    }

    int fds[2];
    if (pipe(fds) != 0) {
        return xr_err("cannot create a pipe for std::process::exec");
    }
    pid_t pid = fork();
    if (pid < 0) {
        close(fds[0]);
        close(fds[1]);
        return xr_err("cannot fork for std::process::exec");
    }
    if (pid == 0) {
        dup2(fds[1], 1);
        dup2(fds[1], 2);
        close(fds[0]);
        close(fds[1]);
        execvp(exe->data, argv);
        _exit(127);
    }
    close(fds[1]);
    long long bytes = 0;
    long long buffer_cap = 4096;
    char *data = (char *)xr_alloc((size_t)buffer_cap);
    ssize_t got = 0;
    while ((got = read(fds[0], data + bytes, (size_t)(buffer_cap - bytes - 1))) > 0) {
        bytes += (long long)got;
        if (bytes + 1 >= buffer_cap) {
            long long next_cap = buffer_cap * 2;
            char *next = (char *)xr_alloc((size_t)next_cap);
            memcpy(next, data, (size_t)bytes);
            data = next;
            buffer_cap = next_cap;
        }
    }
    data[bytes] = '\0';
    close(fds[0]);
    int status = 0;
    waitpid(pid, &status, 0);
    long long code = WIFEXITED(status) ? (long long)WEXITSTATUS(status) : -1;
    return xr_run_result(data, bytes, code);
}

#endif
