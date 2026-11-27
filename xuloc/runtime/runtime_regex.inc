// ── std::regex ───────────────────────────────────────────────────────────
//
// `RegExp::compile` builds a small backtracking engine for the feature set of
// `std-spec/modules/regex.md`: literals, `.`, character classes (including
// `\d`/`\w`/`\s` and their negations), groups (capturing, non-capturing,
// named), alternation, the greedy/lazy repetition operators, and the `^`/`$`/
// `\b` anchors. Look-around and back-references are refused, as the spec
// requires. Matching walks Unicode scalars; `\d`/`\w`/`\s` approximate the
// engine's Unicode classes over ASCII and treat other scalars as word text.

typedef struct {
    int neg;
    int count;
    int cap;
    unsigned int *los;
    unsigned int *his;
} XrRxItem;

typedef struct {
    int negated;
    int count;
    int cap;
    XrRxItem *items;
} XrRxClass;

enum {
    XR_RX_EMPTY,
    XR_RX_CHAR,
    XR_RX_ANY,
    XR_RX_CLASS,
    XR_RX_BOL,
    XR_RX_EOL,
    XR_RX_WORDB,
    XR_RX_NWORDB,
    XR_RX_CONCAT,
    XR_RX_ALT,
    XR_RX_REPEAT,
    XR_RX_GROUP
};

typedef struct XrRxNode XrRxNode;
struct XrRxNode {
    int kind;
    unsigned int ch;
    int min;
    int max;
    int greedy;
    int group;
    char *name;
    XrRxClass *cls;
    XrRxNode **kids;
    int nkids;
    int kid_cap;
    XrRxNode *child;
};

enum { XR_RXI_CHAR, XR_RXI_ANY, XR_RXI_CLASS, XR_RXI_BOL, XR_RXI_EOL,
       XR_RXI_WORDB, XR_RXI_NWORDB, XR_RXI_SAVE, XR_RXI_SPLIT, XR_RXI_JMP,
       XR_RXI_MATCH };

typedef struct {
    int op;
    unsigned int ch;
    int a;
    int b;
    int slot;
    XrRxClass *cls;
} XrRxIns;

typedef struct {
    XrRxIns *ins;
    int len;
    int cap;
    int ngroups;
    int flags_i;
    int flags_m;
    int flags_s;
    char **names;
} XrRxProg;

typedef struct {
    const char *pat;
    long long len;
    long long at;
    int flags_i;
    int flags_m;
    int flags_s;
    int ngroups;
    char **names;
    int name_cap;
    int error;
    long long caret;
    char message[128];
} XrRxParser;

typedef struct {
    XrRxProg prog;
    int has_g;
} XrRegex;

static XrRegex **xr_regex_table = NULL;
static long long xr_regex_count = 0;
static long long xr_regex_cap = 0;

static void *xr_rx_grow(void *old, long long old_count, long long new_count, size_t item) {
    void *next = xr_alloc((size_t)new_count * item);
    if (old != NULL && old_count > 0) {
        memcpy(next, old, (size_t)old_count * item);
    }
    return next;
}

static unsigned int xr_rx_toggle(unsigned int c) {
    if (c >= 'A' && c <= 'Z') {
        return c + 32;
    }
    if (c >= 'a' && c <= 'z') {
        return c - 32;
    }
    return c;
}

static int xr_rx_is_word(unsigned int c) {
    return (c >= '0' && c <= '9') || (c >= 'A' && c <= 'Z') || c == '_' ||
           (c >= 'a' && c <= 'z') || c >= 0x80;
}

static void xr_rx_add_range(XrRxItem *item, unsigned int lo, unsigned int hi) {
    if (item->count == item->cap) {
        int next = item->cap == 0 ? 4 : item->cap * 2;
        item->los = (unsigned int *)xr_rx_grow(item->los, item->count, next, sizeof(unsigned int));
        item->his = (unsigned int *)xr_rx_grow(item->his, item->count, next, sizeof(unsigned int));
        item->cap = next;
    }
    item->los[item->count] = lo;
    item->his[item->count] = hi;
    item->count += 1;
}

static XrRxItem *xr_rx_new_item(int neg) {
    XrRxItem *item = (XrRxItem *)xr_alloc(sizeof(XrRxItem));
    item->neg = neg;
    return item;
}

static void xr_rx_add_item(XrRxClass *cls, XrRxItem *item) {
    if (cls->count == cls->cap) {
        int next = cls->cap == 0 ? 4 : cls->cap * 2;
        cls->items = (XrRxItem *)xr_rx_grow(cls->items, cls->count, next, sizeof(XrRxItem));
        cls->cap = next;
    }
    cls->items[cls->count] = *item;
    cls->count += 1;
}

static void xr_rx_digit_item(XrRxClass *cls, int neg) {
    XrRxItem *item = xr_rx_new_item(neg);
    xr_rx_add_range(item, '0', '9');
    xr_rx_add_item(cls, item);
}

static void xr_rx_word_item(XrRxClass *cls, int neg) {
    XrRxItem *item = xr_rx_new_item(neg);
    xr_rx_add_range(item, '0', '9');
    xr_rx_add_range(item, 'A', 'Z');
    xr_rx_add_range(item, '_', '_');
    xr_rx_add_range(item, 'a', 'z');
    xr_rx_add_range(item, 0x80, 0x10FFFF);
    xr_rx_add_item(cls, item);
}

static void xr_rx_space_item(XrRxClass *cls, int neg) {
    XrRxItem *item = xr_rx_new_item(neg);
    xr_rx_add_range(item, 0x09, 0x0D);
    xr_rx_add_range(item, 0x20, 0x20);
    xr_rx_add_range(item, 0xA0, 0xA0);
    xr_rx_add_range(item, 0x1680, 0x1680);
    xr_rx_add_range(item, 0x2000, 0x200A);
    xr_rx_add_range(item, 0x2028, 0x2029);
    xr_rx_add_range(item, 0x202F, 0x202F);
    xr_rx_add_range(item, 0x205F, 0x205F);
    xr_rx_add_range(item, 0x3000, 0x3000);
    xr_rx_add_item(cls, item);
}

static int xr_rx_item_matches(XrRxItem *item, unsigned int c) {
    int inside = 0;
    for (int i = 0; i < item->count; i += 1) {
        if (c >= item->los[i] && c <= item->his[i]) {
            inside = 1;
            break;
        }
    }
    return item->neg ? !inside : inside;
}

static int xr_rx_class_matches(XrRxClass *cls, unsigned int c, int fold) {
    unsigned int alt = fold ? xr_rx_toggle(c) : c;
    int any = 0;
    for (int i = 0; i < cls->count; i += 1) {
        int hit = xr_rx_item_matches(&cls->items[i], c);
        if (!hit && fold && alt != c) {
            hit = xr_rx_item_matches(&cls->items[i], alt);
        }
        if (hit) {
            any = 1;
            break;
        }
    }
    return cls->negated ? !any : any;
}

// ── parser ───────────────────────────────────────────────────────────────

static XrRxNode *xr_rx_node(int kind) {
    XrRxNode *node = (XrRxNode *)xr_alloc(sizeof(XrRxNode));
    node->kind = kind;
    node->group = -1;
    node->min = 0;
    node->max = 0;
    node->greedy = 1;
    return node;
}

static void xr_rx_add_kid(XrRxNode *node, XrRxNode *kid) {
    if (node->nkids == node->kid_cap) {
        int next = node->kid_cap == 0 ? 2 : node->kid_cap * 2;
        node->kids = (XrRxNode **)xr_rx_grow(node->kids, node->nkids, next, sizeof(XrRxNode *));
        node->kid_cap = next;
    }
    node->kids[node->nkids] = kid;
    node->nkids += 1;
}

static XrRxNode *xr_rx_error_at(XrRxParser *p, long long caret, const char *message) {
    if (!p->error) {
        p->error = 1;
        p->caret = caret;
        snprintf(p->message, sizeof(p->message), "%s", message);
    }
    return xr_rx_node(XR_RX_EMPTY);
}

static int xr_rx_peek(XrRxParser *p, unsigned int *out, int *bytes) {
    if (p->at >= p->len) {
        return 0;
    }
    int n = (int)xr_utf8_len((const unsigned char *)p->pat + p->at);
    if (p->at + n > p->len) {
        n = 1;
    }
    *out = xr_utf8_decode((const unsigned char *)p->pat + p->at, n);
    *bytes = n;
    return 1;
}

static unsigned int xr_rx_take(XrRxParser *p, int *bytes) {
    unsigned int c = 0;
    int n = 0;
    if (!xr_rx_peek(p, &c, &n)) {
        *bytes = 0;
        return 0;
    }
    p->at += n;
    if (bytes != NULL) {
        *bytes = n;
    }
    return c;
}

static void xr_rx_remember_name(XrRxParser *p, int group, const char *name, long long bytes) {
    if (group > p->name_cap) {
        int next = group + 8;
        p->names = (char **)xr_rx_grow(p->names, p->name_cap, next, sizeof(char *));
        p->name_cap = next;
    }
    char *copy = xr_copy_bytes(name, bytes);
    p->names[group - 1] = copy;
}

static int xr_rx_hex_value(unsigned int c) {
    if (c >= '0' && c <= '9') {
        return (int)(c - '0');
    }
    if (c >= 'a' && c <= 'f') {
        return (int)(c - 'a') + 10;
    }
    if (c >= 'A' && c <= 'F') {
        return (int)(c - 'A') + 10;
    }
    return -1;
}

static XrRxNode *xr_rx_parse_alt(XrRxParser *p);

static XrRxNode *xr_rx_parse_escape(XrRxParser *p, int in_class, unsigned int *literal,
                                    int *is_literal) {
    long long caret = p->at;
    p->at += 1; // '\\'
    if (p->at >= p->len) {
        return xr_rx_error_at(p, caret, "incomplete escape sequence");
    }
    unsigned int c = 0;
    c = xr_rx_take(p, NULL);
    *is_literal = 0;
    if (c == 'd' || c == 'D' || c == 'w' || c == 'W' || c == 's' || c == 'S') {
        XrRxClass *cls = (XrRxClass *)xr_alloc(sizeof(XrRxClass));
        if (c == 'd' || c == 'D') {
            xr_rx_digit_item(cls, c == 'D');
        } else if (c == 'w' || c == 'W') {
            xr_rx_word_item(cls, c == 'W');
        } else {
            xr_rx_space_item(cls, c == 'S');
        }
        XrRxNode *node = xr_rx_node(XR_RX_CLASS);
        node->cls = cls;
        return node;
    }
    if (c == 'n') { *literal = '\n'; *is_literal = 1; return NULL; }
    if (c == 'r') { *literal = '\r'; *is_literal = 1; return NULL; }
    if (c == 't') { *literal = '\t'; *is_literal = 1; return NULL; }
    if (c == 'f') { *literal = '\f'; *is_literal = 1; return NULL; }
    if (c == 'v') { *literal = '\v'; *is_literal = 1; return NULL; }
    if (c == '0') { *literal = 0; *is_literal = 1; return NULL; }
    if (c == 'b' && !in_class) {
        return xr_rx_node(XR_RX_WORDB);
    }
    if (c == 'B' && !in_class) {
        return xr_rx_node(XR_RX_NWORDB);
    }
    if (c == 'A' && !in_class) {
        return xr_rx_node(XR_RX_BOL);
    }
    if (c == 'z' && !in_class) {
        return xr_rx_node(XR_RX_EOL);
    }
    if (c == 'x') {
        unsigned int value = 0;
        int digits = 0;
        if (p->at < p->len && p->pat[p->at] == '{') {
            p->at += 1;
            while (p->at < p->len && p->pat[p->at] != '}') {
                int d = xr_rx_hex_value((unsigned char)p->pat[p->at]);
                if (d < 0) {
                    return xr_rx_error_at(p, caret, "invalid hexadecimal escape");
                }
                value = value * 16 + (unsigned int)d;
                digits += 1;
                p->at += 1;
            }
            if (p->at >= p->len || p->pat[p->at] != '}') {
                return xr_rx_error_at(p, caret, "unclosed hexadecimal escape");
            }
            p->at += 1;
        } else {
            while (digits < 2 && p->at < p->len) {
                int d = xr_rx_hex_value((unsigned char)p->pat[p->at]);
                if (d < 0) {
                    break;
                }
                value = value * 16 + (unsigned int)d;
                digits += 1;
                p->at += 1;
            }
        }
        if (digits == 0 || value > 0x10FFFF) {
            return xr_rx_error_at(p, caret, "invalid hexadecimal escape");
        }
        *literal = value;
        *is_literal = 1;
        return NULL;
    }
    if ((c >= '1' && c <= '9') && !in_class) {
        return xr_rx_error_at(p, caret, "backreferences are not supported");
    }
    // Punctuation and other ASCII characters escape to themselves.
    if (c < 0x80) {
        *literal = c;
        *is_literal = 1;
        return NULL;
    }
    return xr_rx_error_at(p, caret, "unrecognized escape sequence");
}

static XrRxNode *xr_rx_parse_class(XrRxParser *p) {
    long long start = p->at;
    p->at += 1; // '['
    XrRxClass *cls = (XrRxClass *)xr_alloc(sizeof(XrRxClass));
    if (p->at < p->len && p->pat[p->at] == '^') {
        cls->negated = 1;
        p->at += 1;
    }
    int first = 1;
    while (p->at < p->len) {
        if (p->pat[p->at] == ']' && !first) {
            p->at += 1;
            XrRxNode *node = xr_rx_node(XR_RX_CLASS);
            node->cls = cls;
            return node;
        }
        first = 0;
        unsigned int lo = 0;
        int have_lo = 0;
        if (p->pat[p->at] == '\\') {
            unsigned int literal = 0;
            int is_literal = 0;
            XrRxNode *escaped = xr_rx_parse_escape(p, 1, &literal, &is_literal);
            if (p->error) {
                return escaped;
            }
            if (escaped != NULL) {
                if (escaped->kind == XR_RX_CLASS) {
                    for (int i = 0; i < escaped->cls->count; i += 1) {
                        xr_rx_add_item(cls, &escaped->cls->items[i]);
                    }
                    continue;
                }
                return xr_rx_error_at(p, start, "unsupported class escape");
            }
            lo = literal;
            have_lo = 1;
        } else {
            lo = xr_rx_take(p, NULL);
            have_lo = 1;
        }
        if (!have_lo) {
            continue;
        }
        if (p->at < p->len && p->pat[p->at] == '-' && p->at + 1 < p->len &&
            p->pat[p->at + 1] != ']') {
            p->at += 1;
            unsigned int hi = 0;
            if (p->pat[p->at] == '\\') {
                unsigned int literal = 0;
                int is_literal = 0;
                XrRxNode *escaped = xr_rx_parse_escape(p, 1, &literal, &is_literal);
                if (p->error) {
                    return escaped;
                }
                if (escaped != NULL || !is_literal) {
                    return xr_rx_error_at(p, start, "invalid character class range");
                }
                hi = literal;
            } else {
                hi = xr_rx_take(p, NULL);
            }
            if (hi < lo) {
                return xr_rx_error_at(p, start, "invalid character class range");
            }
            XrRxItem *item = xr_rx_new_item(0);
            xr_rx_add_range(item, lo, hi);
            xr_rx_add_item(cls, item);
        } else {
            XrRxItem *item = xr_rx_new_item(0);
            xr_rx_add_range(item, lo, lo);
            xr_rx_add_item(cls, item);
        }
    }
    return xr_rx_error_at(p, start, "unclosed character class");
}

static XrRxNode *xr_rx_parse_group(XrRxParser *p) {
    long long start = p->at;
    p->at += 1; // '('
    int group = -1;
    char name[128];
    long long name_len = 0;
    if (p->at < p->len && p->pat[p->at] == '?') {
        p->at += 1;
        if (p->at < p->len && p->pat[p->at] == ':') {
            p->at += 1;
        } else if (p->at < p->len && p->pat[p->at] == 'P' && p->at + 1 < p->len &&
                   p->pat[p->at + 1] == '<') {
            p->at += 2;
            while (p->at < p->len && p->pat[p->at] != '>' && name_len < 127) {
                name[name_len++] = p->pat[p->at++];
            }
            if (p->at >= p->len || p->pat[p->at] != '>') {
                return xr_rx_error_at(p, start, "unclosed capture group name");
            }
            p->at += 1;
            group = p->ngroups + 1;
            p->ngroups = group;
            xr_rx_remember_name(p, group, name, name_len);
        } else if (p->at < p->len && p->pat[p->at] == '<') {
            p->at += 1;
            if (p->at < p->len && (p->pat[p->at] == '=' || p->pat[p->at] == '!')) {
                return xr_rx_error_at(p, start, "look-around is not supported");
            }
            while (p->at < p->len && p->pat[p->at] != '>' && name_len < 127) {
                name[name_len++] = p->pat[p->at++];
            }
            if (p->at >= p->len || p->pat[p->at] != '>') {
                return xr_rx_error_at(p, start, "unclosed capture group name");
            }
            p->at += 1;
            group = p->ngroups + 1;
            p->ngroups = group;
            xr_rx_remember_name(p, group, name, name_len);
        } else if (p->at < p->len && (p->pat[p->at] == '=' || p->pat[p->at] == '!')) {
            return xr_rx_error_at(p, start, "look-around is not supported");
        } else {
            return xr_rx_error_at(p, start, "unsupported group syntax");
        }
    } else {
        group = p->ngroups + 1;
        p->ngroups = group;
    }
    XrRxNode *node = xr_rx_node(XR_RX_GROUP);
    node->group = group;
    if (group > 0 && name_len > 0) {
        node->name = xr_copy_bytes(name, name_len);
    }
    node->child = xr_rx_parse_alt(p);
    if (p->error) {
        return node;
    }
    if (p->at >= p->len || p->pat[p->at] != ')') {
        return xr_rx_error_at(p, start, "unclosed group");
    }
    p->at += 1;
    return node;
}

static int xr_rx_decimal(XrRxParser *p, long long *out) {
    long long start = p->at;
    long long value = 0;
    while (p->at < p->len && p->pat[p->at] >= '0' && p->pat[p->at] <= '9') {
        value = value * 10 + (p->pat[p->at] - '0');
        if (value > 1000) {
            return 0;
        }
        p->at += 1;
    }
    if (p->at == start) {
        return 0;
    }
    *out = value;
    return 1;
}

static XrRxNode *xr_rx_parse_atom(XrRxParser *p) {
    if (p->at >= p->len) {
        return xr_rx_node(XR_RX_EMPTY);
    }
    char c = p->pat[p->at];
    if (c == '(') {
        return xr_rx_parse_group(p);
    }
    if (c == '[') {
        return xr_rx_parse_class(p);
    }
    if (c == '.') {
        p->at += 1;
        return xr_rx_node(XR_RX_ANY);
    }
    if (c == '^') {
        p->at += 1;
        return xr_rx_node(XR_RX_BOL);
    }
    if (c == '$') {
        p->at += 1;
        return xr_rx_node(XR_RX_EOL);
    }
    if (c == '\\') {
        unsigned int literal = 0;
        int is_literal = 0;
        XrRxNode *node = xr_rx_parse_escape(p, 0, &literal, &is_literal);
        if (p->error) {
            return node;
        }
        if (node != NULL) {
            return node;
        }
        if (!is_literal) {
            return xr_rx_error_at(p, p->at, "unrecognized escape sequence");
        }
        node = xr_rx_node(XR_RX_CHAR);
        node->ch = literal;
        return node;
    }
    if (c == '*' || c == '+' || c == '?') {
        return xr_rx_error_at(p, p->at, "repetition operator missing expression");
    }
    XrRxNode *node = xr_rx_node(XR_RX_CHAR);
    node->ch = xr_rx_take(p, NULL);
    return node;
}

static XrRxNode *xr_rx_parse_repeat(XrRxParser *p) {
    XrRxNode *node = xr_rx_parse_atom(p);
    if (p->error || p->at >= p->len) {
        return node;
    }
    int min = -1;
    int max = -1;
    char c = p->pat[p->at];
    if (c == '*') {
        min = 0;
        max = -1;
        p->at += 1;
    } else if (c == '+') {
        min = 1;
        max = -1;
        p->at += 1;
    } else if (c == '?') {
        min = 0;
        max = 1;
        p->at += 1;
    } else if (c == '{' && p->at + 1 < p->len && p->pat[p->at + 1] >= '0' &&
               p->pat[p->at + 1] <= '9') {
        long long brace = p->at;
        p->at += 1;
        long long low = 0;
        if (!xr_rx_decimal(p, &low)) {
            return xr_rx_error_at(p, brace, "invalid repetition quantifier");
        }
        long long high = low;
        if (p->at < p->len && p->pat[p->at] == ',') {
            p->at += 1;
            if (p->at < p->len && p->pat[p->at] == '}') {
                high = -1;
            } else if (!xr_rx_decimal(p, &high)) {
                return xr_rx_error_at(p, brace, "invalid repetition quantifier");
            }
        }
        if (p->at >= p->len || p->pat[p->at] != '}') {
            return xr_rx_error_at(p, brace, "unclosed repetition quantifier");
        }
        p->at += 1;
        if (high >= 0 && high < low) {
            return xr_rx_error_at(p, brace, "repetition quantifier out of order");
        }
        min = (int)low;
        max = (int)high;
    }
    if (min < 0) {
        return node;
    }
    int greedy = 1;
    if (p->at < p->len && p->pat[p->at] == '?') {
        greedy = 0;
        p->at += 1;
    }
    XrRxNode *repeat = xr_rx_node(XR_RX_REPEAT);
    repeat->min = min;
    repeat->max = max;
    repeat->greedy = greedy;
    repeat->child = node;
    return repeat;
}

static XrRxNode *xr_rx_parse_concat(XrRxParser *p) {
    XrRxNode *cat = xr_rx_node(XR_RX_CONCAT);
    while (p->at < p->len && p->pat[p->at] != ')' && p->pat[p->at] != '|') {
        XrRxNode *node = xr_rx_parse_repeat(p);
        if (p->error) {
            return cat;
        }
        xr_rx_add_kid(cat, node);
    }
    if (cat->nkids == 1) {
        return cat->kids[0];
    }
    return cat;
}

static XrRxNode *xr_rx_parse_alt(XrRxParser *p) {
    XrRxNode *first = xr_rx_parse_concat(p);
    if (p->error || p->at >= p->len || p->pat[p->at] != '|') {
        return first;
    }
    XrRxNode *alt = xr_rx_node(XR_RX_ALT);
    xr_rx_add_kid(alt, first);
    while (p->at < p->len && p->pat[p->at] == '|') {
        p->at += 1;
        XrRxNode *next = xr_rx_parse_concat(p);
        if (p->error) {
            return alt;
        }
        xr_rx_add_kid(alt, next);
    }
    return alt;
}

// ── compiler ─────────────────────────────────────────────────────────────

static int xr_rx_emit(XrRxProg *p, int op, unsigned int ch, int a, int b, int slot,
                      XrRxClass *cls) {
    if (p->len == p->cap) {
        int next = p->cap == 0 ? 32 : p->cap * 2;
        p->ins = (XrRxIns *)xr_rx_grow(p->ins, p->len, next, sizeof(XrRxIns));
        p->cap = next;
    }
    XrRxIns *ins = &p->ins[p->len];
    ins->op = op;
    ins->ch = ch;
    ins->a = a;
    ins->b = b;
    ins->slot = slot;
    ins->cls = cls;
    p->len += 1;
    return p->len - 1;
}

static void xr_rx_compile_node(XrRxProg *p, XrRxNode *node);

static void xr_rx_compile_alt(XrRxProg *p, XrRxNode *node) {
    if (node->nkids == 1) {
        xr_rx_compile_node(p, node->kids[0]);
        return;
    }
    int *jumps = (int *)xr_alloc(sizeof(int) * (size_t)node->nkids);
    int jump_count = 0;
    for (int i = 0; i < node->nkids; i += 1) {
        int split = xr_rx_emit(p, XR_RXI_SPLIT, 0, 0, 0, 0, NULL);
        p->ins[split].a = p->len;
        xr_rx_compile_node(p, node->kids[i]);
        if (i < node->nkids - 1) {
            int jump = xr_rx_emit(p, XR_RXI_JMP, 0, 0, 0, 0, NULL);
            jumps[jump_count++] = jump;
        }
        p->ins[split].b = p->len;
    }
    for (int i = 0; i < jump_count; i += 1) {
        p->ins[jumps[i]].a = p->len;
    }
}

static void xr_rx_compile_repeat(XrRxProg *p, XrRxNode *node) {
    for (int i = 0; i < node->min; i += 1) {
        xr_rx_compile_node(p, node->child);
    }
    int unbounded = node->max < 0;
    int optional = unbounded ? 0 : node->max - node->min;
    if (unbounded) {
        int split = xr_rx_emit(p, XR_RXI_SPLIT, 0, 0, 0, 0, NULL);
        int body = p->len;
        xr_rx_compile_node(p, node->child);
        xr_rx_emit(p, XR_RXI_JMP, 0, split, 0, 0, NULL);
        int after = p->len;
        if (node->greedy) {
            p->ins[split].a = body;
            p->ins[split].b = after;
        } else {
            p->ins[split].a = after;
            p->ins[split].b = body;
        }
        return;
    }
    for (int i = 0; i < optional; i += 1) {
        int split = xr_rx_emit(p, XR_RXI_SPLIT, 0, 0, 0, 0, NULL);
        int body = p->len;
        xr_rx_compile_node(p, node->child);
        int after = p->len;
        if (node->greedy) {
            p->ins[split].a = body;
            p->ins[split].b = after;
        } else {
            p->ins[split].a = after;
            p->ins[split].b = body;
        }
    }
}

static void xr_rx_compile_node(XrRxProg *p, XrRxNode *node) {
    switch (node->kind) {
    case XR_RX_EMPTY:
        return;
    case XR_RX_CHAR:
        xr_rx_emit(p, XR_RXI_CHAR, node->ch, 0, 0, 0, NULL);
        return;
    case XR_RX_ANY:
        xr_rx_emit(p, XR_RXI_ANY, 0, 0, 0, 0, NULL);
        return;
    case XR_RX_CLASS:
        xr_rx_emit(p, XR_RXI_CLASS, 0, 0, 0, 0, node->cls);
        return;
    case XR_RX_BOL:
        xr_rx_emit(p, XR_RXI_BOL, 0, 0, 0, 0, NULL);
        return;
    case XR_RX_EOL:
        xr_rx_emit(p, XR_RXI_EOL, 0, 0, 0, 0, NULL);
        return;
    case XR_RX_WORDB:
        xr_rx_emit(p, XR_RXI_WORDB, 0, 0, 0, 0, NULL);
        return;
    case XR_RX_NWORDB:
        xr_rx_emit(p, XR_RXI_NWORDB, 0, 0, 0, 0, NULL);
        return;
    case XR_RX_CONCAT:
        for (int i = 0; i < node->nkids; i += 1) {
            xr_rx_compile_node(p, node->kids[i]);
        }
        return;
    case XR_RX_ALT:
        xr_rx_compile_alt(p, node);
        return;
    case XR_RX_REPEAT:
        xr_rx_compile_repeat(p, node);
        return;
    case XR_RX_GROUP:
        if (node->group > 0) {
            xr_rx_emit(p, XR_RXI_SAVE, 0, 0, 0, node->group * 2, NULL);
        }
        xr_rx_compile_node(p, node->child);
        if (node->group > 0) {
            xr_rx_emit(p, XR_RXI_SAVE, 0, 0, 0, node->group * 2 + 1, NULL);
        }
        return;
    }
}

// ── matcher ──────────────────────────────────────────────────────────────

static long long xr_rx_run(XrRxProg *p, int pc, const char *s, long long len, long long pos,
                           long long *caps, long long *steps) {
    if (*steps > 10000000) {
        return -1;
    }
    *steps += 1;
    XrRxIns *ins = &p->ins[pc];
    switch (ins->op) {
    case XR_RXI_CHAR: {
        if (pos >= len) {
            return -1;
        }
        int n = (int)xr_utf8_len((const unsigned char *)s + pos);
        if (pos + n > len) {
            n = 1;
        }
        unsigned int c = xr_utf8_decode((const unsigned char *)s + pos, n);
        unsigned int want = ins->ch;
        if (p->flags_i) {
            c = xr_rx_toggle(c);
            want = xr_rx_toggle(want);
        }
        if (c != want) {
            return -1;
        }
        return xr_rx_run(p, pc + 1, s, len, pos + n, caps, steps);
    }
    case XR_RXI_ANY: {
        if (pos >= len) {
            return -1;
        }
        int n = (int)xr_utf8_len((const unsigned char *)s + pos);
        if (pos + n > len) {
            n = 1;
        }
        if (!p->flags_s && s[pos] == '\n') {
            return -1;
        }
        return xr_rx_run(p, pc + 1, s, len, pos + n, caps, steps);
    }
    case XR_RXI_CLASS: {
        if (pos >= len) {
            return -1;
        }
        int n = (int)xr_utf8_len((const unsigned char *)s + pos);
        if (pos + n > len) {
            n = 1;
        }
        unsigned int c = xr_utf8_decode((const unsigned char *)s + pos, n);
        if (!xr_rx_class_matches(ins->cls, c, p->flags_i)) {
            return -1;
        }
        return xr_rx_run(p, pc + 1, s, len, pos + n, caps, steps);
    }
    case XR_RXI_BOL: {
        int ok = pos == 0 || (p->flags_m && s[pos - 1] == '\n');
        return ok ? xr_rx_run(p, pc + 1, s, len, pos, caps, steps) : -1;
    }
    case XR_RXI_EOL: {
        int ok = pos == len || (p->flags_m && s[pos] == '\n');
        return ok ? xr_rx_run(p, pc + 1, s, len, pos, caps, steps) : -1;
    }
    case XR_RXI_WORDB:
    case XR_RXI_NWORDB: {
        int before = 0;
        int after = 0;
        if (pos > 0) {
            int n = (int)xr_utf8_len((const unsigned char *)s + pos - 1);
            before = xr_rx_is_word(xr_utf8_decode((const unsigned char *)s + pos - 1, n));
        }
        if (pos < len) {
            int n = (int)xr_utf8_len((const unsigned char *)s + pos);
            if (pos + n > len) {
                n = 1;
            }
            after = xr_rx_is_word(xr_utf8_decode((const unsigned char *)s + pos, n));
        }
        int boundary = before != after;
        if (ins->op == XR_RXI_NWORDB) {
            boundary = !boundary;
        }
        return boundary ? xr_rx_run(p, pc + 1, s, len, pos, caps, steps) : -1;
    }
    case XR_RXI_SAVE: {
        long long old = caps[ins->slot];
        caps[ins->slot] = pos;
        long long result = xr_rx_run(p, pc + 1, s, len, pos, caps, steps);
        if (result < 0) {
            caps[ins->slot] = old;
        }
        return result;
    }
    case XR_RXI_SPLIT: {
        long long result = xr_rx_run(p, ins->a, s, len, pos, caps, steps);
        if (result >= 0) {
            return result;
        }
        return xr_rx_run(p, ins->b, s, len, pos, caps, steps);
    }
    case XR_RXI_JMP:
        return xr_rx_run(p, ins->a, s, len, pos, caps, steps);
    case XR_RXI_MATCH:
        return pos;
    }
    return -1;
}

static int xr_rx_search(XrRxProg *p, const char *s, long long len, long long from,
                        long long *caps) {
    long long cap_count = (long long)(p->ngroups + 1) * 2;
    for (long long at = from; at <= len;) {
        for (long long i = 0; i < cap_count; i += 1) {
            caps[i] = -1;
        }
        long long steps = 0;
        long long end = xr_rx_run(p, 0, s, len, at, caps, &steps);
        if (end >= 0) {
            if (caps[0] < 0) {
                caps[0] = at;
            }
            if (caps[1] < 0) {
                caps[1] = end;
            }
            return 1;
        }
        if (at == len) {
            break;
        }
        int n = (int)xr_utf8_len((const unsigned char *)s + at);
        if (at + n > len) {
            n = 1;
        }
        at += n;
    }
    return 0;
}

static long long xr_rx_scalar_index(const char *s, long long at) {
    return xr_utf8_count(s, at);
}

static XVal xr_rx_make_match(XrRxProg *p, const char *s, long long len, long long *caps) {
    long long start = caps[0] < 0 ? 0 : caps[0];
    long long end = caps[1] < 0 ? start : caps[1];
    const char *names[5];
    XVal values[5];
    names[0] = "value";
    names[1] = "index";
    names[2] = "end";
    names[3] = "groups";
    names[4] = "named";
    values[0] = xr_str_n(s + start, end - start);
    values[1] = xr_int(xr_rx_scalar_index(s, start));
    values[2] = xr_int(xr_rx_scalar_index(s, end));
    XVal groups = xr_list_new();
    for (int g = 1; g <= p->ngroups; g += 1) {
        long long a = caps[g * 2];
        long long b = caps[g * 2 + 1];
        if (a >= 0 && b >= 0) {
            xr_list_push(groups, xr_str_n(s + a, b - a));
        } else {
            xr_list_push(groups, xr_nil());
        }
    }
    values[3] = groups;
    XVal named = xr_map_new();
    for (int g = 1; g <= p->ngroups; g += 1) {
        char *name = g <= (int)p->ngroups && p->names != NULL ? p->names[g - 1] : NULL;
        if (name == NULL) {
            continue;
        }
        long long a = caps[g * 2];
        long long b = caps[g * 2 + 1];
        XVal text = a >= 0 && b >= 0 ? xr_str_n(s + a, b - a) : xr_nil();
        xr_map_set(named, xr_str(name), text);
    }
    values[4] = named;
    return xr_struct("RegexMatch", 5, names, values);
}

static XVal xr_rx_result_ok(XVal value) { return xr_variant("Result", "Ok", 1, &value); }

static XVal xr_rx_result_err(XVal error) {
    return xr_variant("Result", "Err", 1, &error);
}

static XVal xr_rx_error_value(const char *pattern, long long caret, const char *message) {
    char buffer[1024];
    snprintf(buffer, sizeof(buffer), "regex parse error:\n    %s\n    %*s^\nerror: %s", pattern,
             (int)caret, "", message);
    const char *names[1];
    XVal values[1];
    names[0] = "message";
    values[0] = xr_str(buffer);
    return xr_struct("RegexError", 1, names, values);
}

static XrRegex *xr_regex_get(XVal value) {
    if (value.tag != XT_OBJ || strcmp(xr_as_obj(value)->type, "RegExp") != 0) {
        xr_fail("expected a compiled `RegExp`");
    }
    XVal id = xr_get_field(value, "id");
    if (id.tag != XT_INT || id.i < 0 || id.i >= xr_regex_count) {
        xr_fail("invalid compiled `RegExp`");
    }
    return xr_regex_table[id.i];
}

static long long xr_regex_add(XrRxProg prog, int has_g) {
    if (xr_regex_count == xr_regex_cap) {
        long long next = xr_regex_cap == 0 ? 8 : xr_regex_cap * 2;
        xr_regex_table = (XrRegex **)xr_rx_grow(xr_regex_table, xr_regex_cap, next,
                                                sizeof(XrRegex *));
        xr_regex_cap = next;
    }
    XrRegex *entry = (XrRegex *)xr_alloc(sizeof(XrRegex));
    entry->prog = prog;
    entry->has_g = has_g;
    xr_regex_table[xr_regex_count] = entry;
    long long id = xr_regex_count;
    xr_regex_count += 1;
    return id;
}

static XVal xr_regex_object(long long id) {
    const char *names[1];
    XVal values[1];
    names[0] = "id";
    values[0] = xr_int(id);
    return xr_struct("RegExp", 1, names, values);
}

XVal xr_regex_compile(XVal pattern_value, XVal flags_value) {
    if (pattern_value.tag != XT_STR) {
        return xr_rx_result_err(xr_rx_error_value("", 0, "pattern must be a string"));
    }
    if (flags_value.tag == XT_NIL) {
        flags_value = xr_str("");
    }
    if (flags_value.tag != XT_STR) {
        return xr_rx_result_err(xr_rx_error_value("", 0, "flags must be a string"));
    }
    XStr *pattern = xr_as_str(pattern_value);
    XStr *flags = xr_as_str(flags_value);
    int has_g = 0;
    for (long long i = 0; i < flags->bytes; i += 1) {
        char flag = flags->data[i];
        if (flag == 'g') {
            has_g = 1;
        } else if (flag == 'i' || flag == 'm' || flag == 'u' || flag == 's') {
            // handled below
        } else {
            char message[64];
            snprintf(message, sizeof(message), "unknown flag `%c`", flag);
            const char *names[1];
            XVal values[1];
            names[0] = "message";
            values[0] = xr_str(message);
            return xr_rx_result_err(xr_struct("RegexError", 1, names, values));
        }
    }
    XrRxParser parser;
    memset(&parser, 0, sizeof(parser));
    parser.pat = pattern->data;
    parser.len = pattern->bytes;
    for (long long i = 0; i < flags->bytes; i += 1) {
        if (flags->data[i] == 'i') {
            parser.flags_i = 1;
        } else if (flags->data[i] == 'm') {
            parser.flags_m = 1;
        } else if (flags->data[i] == 's') {
            parser.flags_s = 1;
        }
    }
    XrRxNode *root = xr_rx_parse_alt(&parser);
    if (!parser.error && parser.at < parser.len) {
        xr_rx_error_at(&parser, parser.at, "unmatched closing parenthesis");
    }
    if (parser.error) {
        return xr_rx_result_err(xr_rx_error_value(pattern->data, parser.caret, parser.message));
    }
    XrRxProg prog;
    memset(&prog, 0, sizeof(prog));
    prog.ngroups = parser.ngroups;
    prog.flags_i = parser.flags_i;
    prog.flags_m = parser.flags_m;
    prog.flags_s = parser.flags_s;
    prog.names = parser.names;
    xr_rx_emit(&prog, XR_RXI_SAVE, 0, 0, 0, 0, NULL);
    xr_rx_compile_node(&prog, root);
    xr_rx_emit(&prog, XR_RXI_SAVE, 0, 0, 0, 1, NULL);
    xr_rx_emit(&prog, XR_RXI_MATCH, 0, 0, 0, 0, NULL);
    long long id = xr_regex_add(prog, has_g);
    return xr_rx_result_ok(xr_regex_object(id));
}

XVal xr_regex_method(XVal recv, const char *name, long long n, XVal *args) {
    XrRegex *entry = xr_regex_get(recv);
    if (n != 1 || args[0].tag != XT_STR) {
        xr_fail("`RegExp` method expects a string input");
    }
    XStr *input = xr_as_str(args[0]);
    long long cap_count = (long long)(entry->prog.ngroups + 1) * 2;
    long long *caps = (long long *)xr_alloc(sizeof(long long) * (size_t)cap_count);
    if (strcmp(name, "test") == 0) {
        return xr_bool(xr_rx_search(&entry->prog, input->data, input->bytes, 0, caps));
    }
    if (strcmp(name, "find") == 0) {
        if (!xr_rx_search(&entry->prog, input->data, input->bytes, 0, caps)) {
            return xr_nil();
        }
        return xr_rx_make_match(&entry->prog, input->data, input->bytes, caps);
    }
    if (strcmp(name, "find_all") == 0) {
        XVal out = xr_list_new();
        long long search = 0;
        long long last_end = -1;
        int prev_nonempty = 0;
        while (search <= input->bytes) {
            if (!xr_rx_search(&entry->prog, input->data, input->bytes, search, caps)) {
                break;
            }
            long long start = caps[0];
            long long end = caps[1];
            if (!(prev_nonempty && start == last_end && start == end)) {
                xr_list_push(out, xr_rx_make_match(&entry->prog, input->data, input->bytes, caps));
                last_end = end;
                prev_nonempty = end > start;
            }
            if (end > start) {
                search = end;
            } else if (end < input->bytes) {
                int step = (int)xr_utf8_len((const unsigned char *)input->data + end);
                if (end + step > input->bytes) {
                    step = 1;
                }
                search = end + step;
            } else {
                search = input->bytes + 1;
            }
        }
        return out;
    }
    xr_fail("unknown `RegExp` method");
    return xr_nil();
}

XVal xr_regex_match_method(XVal recv, const char *name, long long n, XVal *args) {
    if (strcmp(name, "group") != 0) {
        xr_fail("unknown `RegexMatch` method");
    }
    if (n != 1) {
        xr_fail("`RegexMatch.group` expects one argument");
    }
    XVal groups = xr_get_field(recv, "groups");
    XVal named = xr_get_field(recv, "named");
    if (args[0].tag == XT_INT) {
        long long index = args[0].i;
        if (index < 1 || groups.tag != XT_LIST || index > xr_as_list(groups)->len) {
            return xr_nil();
        }
        return xr_as_list(groups)->items[index - 1];
    }
    if (args[0].tag == XT_STR) {
        return xr_map_get(named, args[0]);
    }
    return xr_nil();
}

static void xr_rx_append_group(XrRxProg *p, const char *s, long long *caps, long long group,
                               char **out, long long *len, long long *cap) {
    if (group < 0 || group > p->ngroups) {
        return;
    }
    long long a = caps[group * 2];
    long long b = caps[group * 2 + 1];
    if (a < 0 || b < 0) {
        return;
    }
    xr_append_str(out, len, cap, s + a, b - a);
}

static long long xr_rx_named_group(XrRxProg *p, const char *name, long long bytes) {
    for (int g = 1; g <= p->ngroups; g += 1) {
        char *candidate = p->names != NULL ? p->names[g - 1] : NULL;
        if (candidate != NULL && (long long)strlen(candidate) == bytes &&
            memcmp(candidate, name, (size_t)bytes) == 0) {
            return g;
        }
    }
    return -1;
}

static void xr_rx_expand_template(XrRxProg *p, const char *s, long long *caps, const char *tpl,
                                  long long bytes, char **out, long long *len, long long *cap) {
    long long at = 0;
    while (at < bytes) {
        if (tpl[at] != '$') {
            xr_append_str(out, len, cap, tpl + at, 1);
            at += 1;
            continue;
        }
        at += 1;
        if (at < bytes && tpl[at] == '$') {
            xr_append_str(out, len, cap, "$", 1);
            at += 1;
            continue;
        }
        if (at < bytes && tpl[at] == '{') {
            long long start = at + 1;
            at += 1;
            while (at < bytes && tpl[at] != '}') {
                at += 1;
            }
            if (at >= bytes) {
                xr_append_str(out, len, cap, "${", 2);
                at = start;
                continue;
            }
            long long group = xr_rx_named_group(p, tpl + start, at - start);
            at += 1;
            if (group >= 0) {
                xr_rx_append_group(p, s, caps, group, out, len, cap);
            }
            continue;
        }
        if (at < bytes && tpl[at] >= '0' && tpl[at] <= '9') {
            long long group = 0;
            while (at < bytes && tpl[at] >= '0' && tpl[at] <= '9') {
                group = group * 10 + (tpl[at] - '0');
                at += 1;
            }
            xr_rx_append_group(p, s, caps, group, out, len, cap);
            continue;
        }
        if (at < bytes && ((tpl[at] >= 'A' && tpl[at] <= 'Z') ||
                           (tpl[at] >= 'a' && tpl[at] <= 'z') || tpl[at] == '_')) {
            long long start = at;
            while (at < bytes && ((tpl[at] >= 'A' && tpl[at] <= 'Z') ||
                                  (tpl[at] >= 'a' && tpl[at] <= 'z') ||
                                  (tpl[at] >= '0' && tpl[at] <= '9') || tpl[at] == '_')) {
                at += 1;
            }
            long long group = xr_rx_named_group(p, tpl + start, at - start);
            if (group >= 0) {
                xr_rx_append_group(p, s, caps, group, out, len, cap);
            }
            continue;
        }
        xr_append_str(out, len, cap, "$", 1);
    }
}

XVal xr_regex_replace(XVal regex_value, XVal input_value, XVal replacer) {
    XrRegex *entry = xr_regex_get(regex_value);
    if (input_value.tag != XT_STR) {
        xr_fail("`.replace` expects a string receiver");
    }
    if (replacer.tag != XT_STR && replacer.tag != XT_FN) {
        xr_fail("`.replace` expects a string or a function replacer");
    }
    XStr *input = xr_as_str(input_value);
    long long cap_count = (long long)(entry->prog.ngroups + 1) * 2;
    long long *caps = (long long *)xr_alloc(sizeof(long long) * (size_t)cap_count);
    char *out = NULL;
    long long out_len = 0;
    long long out_cap = 0;
    long long last = 0;
    long long search = 0;
    long long last_end = -1;
    int prev_nonempty = 0;
    int replaced = 0;
    while (search <= input->bytes) {
        if (!xr_rx_search(&entry->prog, input->data, input->bytes, search, caps)) {
            break;
        }
        if (replaced && !entry->has_g) {
            break;
        }
        long long start = caps[0];
        long long end = caps[1];
        int skip = prev_nonempty && start == last_end && start == end;
        if (!skip) {
            xr_append_str(&out, &out_len, &out_cap, input->data + last, start - last);
            if (replacer.tag == XT_STR) {
                XStr *tpl = xr_as_str(replacer);
                xr_rx_expand_template(&entry->prog, input->data, caps, tpl->data, tpl->bytes, &out,
                                      &out_len, &out_cap);
            } else {
                XVal match = xr_rx_make_match(&entry->prog, input->data, input->bytes, caps);
                XVal text = xr_call_value(replacer, 1, &match);
                if (text.tag != XT_STR) {
                    xr_fail("replacement function must return a `string`");
                }
                XStr *str = xr_as_str(text);
                xr_append_str(&out, &out_len, &out_cap, str->data, str->bytes);
            }
            last = end;
            last_end = end;
            prev_nonempty = end > start;
        }
        replaced = 1;
        if (!entry->has_g) {
            break;
        }
        if (end > start) {
            search = end;
        } else if (end < input->bytes) {
            int step = (int)xr_utf8_len((const unsigned char *)input->data + end);
            if (end + step > input->bytes) {
                step = 1;
            }
            search = end + step;
        } else {
            search = input->bytes + 1;
        }
    }
    xr_append_str(&out, &out_len, &out_cap, input->data + last, input->bytes - last);
    return xr_str_n(out == NULL ? "" : out, out_len);
}

XVal xr_regex_split(XVal regex_value, XVal input_value) {
    XrRegex *entry = xr_regex_get(regex_value);
    if (input_value.tag != XT_STR) {
        xr_fail("`.split` expects a string receiver");
    }
    XStr *input = xr_as_str(input_value);
    long long cap_count = (long long)(entry->prog.ngroups + 1) * 2;
    long long *caps = (long long *)xr_alloc(sizeof(long long) * (size_t)cap_count);
    XVal out = xr_list_new();
    long long last = 0;
    long long search = 0;
    long long last_end = -1;
    int prev_nonempty = 0;
    while (search <= input->bytes) {
        if (!xr_rx_search(&entry->prog, input->data, input->bytes, search, caps)) {
            break;
        }
        long long start = caps[0];
        long long end = caps[1];
        if (!(prev_nonempty && start == last_end && start == end)) {
            xr_list_push(out, xr_str_n(input->data + last, start - last));
            last = end;
            last_end = end;
            prev_nonempty = end > start;
        }
        if (end > start) {
            search = end;
        } else if (end < input->bytes) {
            int step = (int)xr_utf8_len((const unsigned char *)input->data + end);
            if (end + step > input->bytes) {
                step = 1;
            }
            search = end + step;
        } else {
            search = input->bytes + 1;
        }
    }
    xr_list_push(out, xr_str_n(input->data + last, input->bytes - last));
    return out;
}
