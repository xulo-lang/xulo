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

