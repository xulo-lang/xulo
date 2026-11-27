// ── arithmetic and comparison ────────────────────────────────────────────

static int xr_is_num(XVal v) { return v.tag == XT_INT || v.tag == XT_FLOAT; }

static double xr_as_double(XVal v) { return v.tag == XT_INT ? (double)v.i : v.f; }

// Integral arithmetic is checked, matching the stage 0 runtime's `xulo_int_*`
// helpers: a result that leaves the 64-bit range is a runtime failure, not a
// silent wrap (spec `types/primitive-types.md`, "Overflow"; drift D-01).
static long long xr_checked_add(long long a, long long b) {
    if ((b > 0 && a > LLONG_MAX - b) || (b < 0 && a < LLONG_MIN - b)) {
        xr_fail("integer overflow");
    }
    return a + b;
}

static long long xr_checked_sub(long long a, long long b) {
    if ((b > 0 && a < LLONG_MIN + b) || (b < 0 && a > LLONG_MAX + b)) {
        xr_fail("integer overflow");
    }
    return a - b;
}

static long long xr_checked_mul(long long a, long long b) {
    if (a == 0 || b == 0) {
        return 0;
    }
    if (a == -1 && b == LLONG_MIN) {
        xr_fail("integer overflow");
    }
    if (b == -1 && a == LLONG_MIN) {
        xr_fail("integer overflow");
    }
    if (a > 0) {
        if (b > 0) {
            if (a > LLONG_MAX / b) {
                xr_fail("integer overflow");
            }
        } else if (b < LLONG_MIN / a) {
            xr_fail("integer overflow");
        }
    } else if (b > 0) {
        if (a < LLONG_MIN / b) {
            xr_fail("integer overflow");
        }
    } else if (a < LLONG_MAX / b) {
        xr_fail("integer overflow");
    }
    return a * b;
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
        return xr_int(xr_checked_add(a.i, b.i));
    }
    if (xr_is_num(a) && xr_is_num(b)) {
        return xr_float(xr_as_double(a) + xr_as_double(b));
    }
    xr_fail("`+` expects numbers, strings, or lists");
    return xr_nil();
}

XVal xr_sub(XVal a, XVal b) {
    if (a.tag == XT_INT && b.tag == XT_INT) {
        return xr_int(xr_checked_sub(a.i, b.i));
    }
    if (xr_is_num(a) && xr_is_num(b)) {
        return xr_float(xr_as_double(a) - xr_as_double(b));
    }
    xr_fail("`-` expects numbers");
    return xr_nil();
}

XVal xr_mul(XVal a, XVal b) {
    if (a.tag == XT_INT && b.tag == XT_INT) {
        return xr_int(xr_checked_mul(a.i, b.i));
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
        if (a.i == LLONG_MIN && b.i == -1) {
            xr_fail("integer overflow");
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
        if (a.i == LLONG_MIN && b.i == -1) {
            xr_fail("integer overflow");
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
                result = xr_checked_mul(result, base);
            }
            base = xr_checked_mul(base, base);
            n >>= 1;
        }
        return xr_int(result);
    }
    return xr_float(pow(xr_as_double(a), xr_as_double(b)));
}

XVal xr_neg(XVal a) {
    if (a.tag == XT_INT) {
        if (a.i == LLONG_MIN) {
            xr_fail("integer overflow");
        }
        return xr_int(-a.i);
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
    case XT_SET: {
        // Sets are unordered: same cardinality, and every element of one is
        // present in the other.
        XSet *x = (XSet *)a.p;
        XSet *y = (XSet *)b.p;
        if (x == y) {
            return 1;
        }
        if (x->len != y->len) {
            return 0;
        }
        for (long long i = 0; i < x->len; i += 1) {
            if (xr_set_find(y, x->items[i]) < 0) {
                return 0;
            }
        }
        return 1;
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
    case XT_FN:
        return a.p == b.p;
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
    if (strcmp(name, "fn") == 0 || strcmp(name, "function") == 0) {
        return v.tag == XT_FN;
    }
    if (v.tag == XT_OBJ) {
        return strcmp(xr_as_obj(v)->type, name) == 0;
    }
    return 0;
}

