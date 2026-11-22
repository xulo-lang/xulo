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

static char *xr_fs_join_path(const char *dir, const char *name) {
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
    char *pattern = xr_fs_join_path(dir, "*");
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
        char *child = xr_fs_join_path(dir, names.items[i]);
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

static const char *xr_fs_path(XVal value) {
    if (value.tag != XT_STR) {
        return NULL;
    }
    return xr_as_str(value)->data;
}

static XVal xr_fs_fail(const char *op, const char *path) {
    char message[600];
    snprintf(message, sizeof(message), "cannot %s `%s`", op, path);
    return xr_err(message);
}

#ifdef _WIN32
typedef struct _stat64 XrStat;
#define XR_STAT _stat64
#define XR_ISDIR(mode) (((mode) & _S_IFDIR) != 0)
#define XR_ISREG(mode) (((mode) & _S_IFREG) != 0)
#else
typedef struct stat XrStat;
#define XR_STAT stat
#define XR_ISDIR(mode) S_ISDIR(mode)
#define XR_ISREG(mode) S_ISREG(mode)
#endif

static int xr_fs_exists_path(const char *path) {
    XrStat info;
    return XR_STAT(path, &info) == 0;
}

static unsigned char *xr_fs_read_all(const char *path, long long *bytes) {
    FILE *file = fopen(path, "rb");
    if (file == NULL) {
        return NULL;
    }
    long long cap = 4096;
    long long len = 0;
    unsigned char *data = (unsigned char *)xr_alloc((size_t)cap);
    size_t got = 0;
    while ((got = fread(data + len, 1, (size_t)(cap - len), file)) > 0) {
        len += (long long)got;
        if (len == cap) {
            cap *= 2;
            unsigned char *next = (unsigned char *)xr_alloc((size_t)cap);
            memcpy(next, data, (size_t)len);
            data = next;
        }
    }
    fclose(file);
    *bytes = len;
    return data;
}

static XVal xr_fs_bytes_value(const unsigned char *data, long long bytes) {
    XVal out = xr_list_new();
    for (long long i = 0; i < bytes; i += 1) {
        xr_list_push(out, xr_int((long long)data[i]));
    }
    return out;
}

static int xr_fs_value_bytes(XVal value, char **data, long long *bytes) {
    if (value.tag != XT_LIST) {
        return 0;
    }
    XList *list = xr_as_list(value);
    char *buffer = (char *)xr_alloc((size_t)list->len + 1);
    for (long long i = 0; i < list->len; i += 1) {
        if (list->items[i].tag != XT_INT) {
            return 0;
        }
        buffer[i] = (char)((unsigned long long)list->items[i].i & 0xFFULL);
    }
    buffer[list->len] = '\0';
    *data = buffer;
    *bytes = list->len;
    return 1;
}

static XVal xr_fs_write_raw(const char *path, const char *data, long long bytes, int append) {
    FILE *file = fopen(path, append ? "ab" : "wb");
    if (file == NULL) {
        return xr_fs_fail(append ? "append to" : "write", path);
    }
    if (bytes > 0) {
        fwrite(data, 1, (size_t)bytes, file);
    }
    fclose(file);
    return xr_ok(xr_nil());
}

XVal xr_fs_read(XVal path) {
    const char *p = xr_fs_path(path);
    if (p == NULL) {
        return xr_err("std::fs::read expects a string path");
    }
    FILE *file = fopen(p, "rb");
    if (file == NULL) {
        return xr_fs_fail("read", p);
    }
    long long bytes = 0;
    char *data = xr_read_stream(file, &bytes);
    fclose(file);
    return xr_ok(xr_str_n(data, bytes));
}

XVal xr_fs_write(XVal path, XVal text) {
    const char *p = xr_fs_path(path);
    if (p == NULL || text.tag != XT_STR) {
        return xr_err("std::fs::write expects a string path and text");
    }
    XStr *s = xr_as_str(text);
    return xr_fs_write_raw(p, s->data, s->bytes, 0);
}

XVal xr_fs_append(XVal path, XVal text) {
    const char *p = xr_fs_path(path);
    if (p == NULL || text.tag != XT_STR) {
        return xr_err("std::fs::append expects a string path and text");
    }
    XStr *s = xr_as_str(text);
    return xr_fs_write_raw(p, s->data, s->bytes, 1);
}

XVal xr_fs_read_bytes(XVal path) {
    const char *p = xr_fs_path(path);
    if (p == NULL) {
        return xr_err("std::fs::read_bytes expects a string path");
    }
    long long bytes = 0;
    unsigned char *data = xr_fs_read_all(p, &bytes);
    if (data == NULL && !xr_fs_exists_path(p)) {
        return xr_fs_fail("read", p);
    }
    return xr_ok(xr_fs_bytes_value(data == NULL ? (const unsigned char *)"" : data, bytes));
}

XVal xr_fs_write_bytes(XVal path, XVal bytes) {
    const char *p = xr_fs_path(path);
    char *data = NULL;
    long long size = 0;
    if (p == NULL || !xr_fs_value_bytes(bytes, &data, &size)) {
        return xr_err("std::fs::write_bytes expects a string path and a List<u8>");
    }
    return xr_fs_write_raw(p, data, size, 0);
}

XVal xr_fs_append_bytes(XVal path, XVal bytes) {
    const char *p = xr_fs_path(path);
    char *data = NULL;
    long long size = 0;
    if (p == NULL || !xr_fs_value_bytes(bytes, &data, &size)) {
        return xr_err("std::fs::append_bytes expects a string path and a List<u8>");
    }
    return xr_fs_write_raw(p, data, size, 1);
}

XVal xr_fs_create_new(XVal path, XVal text) {
    const char *p = xr_fs_path(path);
    if (p == NULL || text.tag != XT_STR) {
        return xr_err("std::fs::create_new expects a string path and text");
    }
    XStr *s = xr_as_str(text);
#ifdef _WIN32
    int fd = _open(p, _O_CREAT | _O_EXCL | _O_WRONLY | _O_BINARY, _S_IREAD | _S_IWRITE);
#else
    int fd = open(p, O_CREAT | O_EXCL | O_WRONLY, 0666);
#endif
    if (fd < 0) {
        return xr_fs_fail("create", p);
    }
    if (s->bytes > 0) {
#ifdef _WIN32
        _write(fd, s->data, (unsigned int)s->bytes);
#else
        ssize_t ignored = write(fd, s->data, (size_t)s->bytes);
        (void)ignored;
#endif
    }
#ifdef _WIN32
    _close(fd);
#else
    close(fd);
#endif
    return xr_ok(xr_nil());
}

XVal xr_fs_exists(XVal path) {
    const char *p = xr_fs_path(path);
    if (p == NULL) {
        return xr_bool(0);
    }
    return xr_bool(xr_fs_exists_path(p));
}

XVal xr_fs_stat(XVal path) {
    const char *p = xr_fs_path(path);
    if (p == NULL) {
        return xr_err("std::fs::stat expects a string path");
    }
    XrStat info;
    if (XR_STAT(p, &info) != 0) {
        return xr_fs_fail("stat", p);
    }
    const char *kind = NULL;
    if (XR_ISDIR(info.st_mode)) {
        kind = "directory";
    } else if (XR_ISREG(info.st_mode)) {
        kind = "file";
    } else {
        return xr_fs_fail("stat", p);
    }
    long long modified = 0;
#ifdef _WIN32
    modified = (long long)info.st_mtime * 1000;
#elif defined(__APPLE__)
    modified = (long long)info.st_mtimespec.tv_sec * 1000 + info.st_mtimespec.tv_nsec / 1000000;
#else
    modified = (long long)info.st_mtim.tv_sec * 1000 + info.st_mtim.tv_nsec / 1000000;
#endif
    if (modified < 0) {
        modified = 0;
    }
    const char *names[4] = {"kind", "size", "modified", "mode"};
    XVal values[4];
    values[0] = xr_str(kind);
    values[1] = xr_int((long long)info.st_size);
    values[2] = xr_int(modified);
    values[3] = xr_int((long long)(info.st_mode & 0777));
    return xr_ok(xr_struct("FileInfo", 4, names, values));
}

XVal xr_fs_copy(XVal from, XVal to) {
    const char *source = xr_fs_path(from);
    const char *target = xr_fs_path(to);
    if (source == NULL || target == NULL) {
        return xr_err("std::fs::copy expects two string paths");
    }
    long long bytes = 0;
    unsigned char *data = xr_fs_read_all(source, &bytes);
    if (data == NULL && !xr_fs_exists_path(source)) {
        return xr_fs_fail("copy", source);
    }
    return xr_fs_write_raw(target, data == NULL ? "" : (const char *)data, bytes, 0);
}

XVal xr_fs_rename(XVal from, XVal to) {
    const char *source = xr_fs_path(from);
    const char *target = xr_fs_path(to);
    if (source == NULL || target == NULL) {
        return xr_err("std::fs::rename expects two string paths");
    }
    if (rename(source, target) != 0) {
        return xr_fs_fail("rename", source);
    }
    return xr_ok(xr_nil());
}

XVal xr_fs_remove(XVal path) {
    const char *p = xr_fs_path(path);
    if (p == NULL) {
        return xr_err("std::fs::remove expects a string path");
    }
#ifdef _WIN32
    int rc = _unlink(p);
#else
    int rc = unlink(p);
#endif
    if (rc != 0) {
        return xr_fs_fail("remove", p);
    }
    return xr_ok(xr_nil());
}

XVal xr_fs_set_mode(XVal path, XVal mode) {
    const char *p = xr_fs_path(path);
    if (p == NULL || mode.tag != XT_INT) {
        return xr_err("std::fs::set_mode expects a path and a number");
    }
#ifdef _WIN32
    (void)p;
    (void)mode;
    return xr_ok(xr_nil());
#else
    if (chmod(p, (mode_t)(mode.i & 0777)) != 0) {
        return xr_fs_fail("set permissions of", p);
    }
    return xr_ok(xr_nil());
#endif
}

XVal xr_fs_set_times(XVal path, XVal modified) {
    const char *p = xr_fs_path(path);
    if (p == NULL || modified.tag != XT_INT) {
        return xr_err("std::fs::set_times expects a path and a number");
    }
#ifdef _WIN32
    struct _utimbuf times;
    times.actime = (time_t)(modified.i / 1000);
    times.modtime = (time_t)(modified.i / 1000);
    if (_utime(p, &times) != 0) {
        return xr_fs_fail("set times of", p);
    }
#else
    struct timeval times[2];
    times[0].tv_sec = (time_t)(modified.i / 1000);
    times[0].tv_usec = (suseconds_t)((modified.i % 1000) * 1000);
    times[1] = times[0];
    if (utimes(p, times) != 0) {
        return xr_fs_fail("set times of", p);
    }
#endif
    return xr_ok(xr_nil());
}

XVal xr_fs_touch(XVal path) {
    const char *p = xr_fs_path(path);
    if (p == NULL) {
        return xr_err("std::fs::touch expects a string path");
    }
#ifdef _WIN32
    if (_utime(p, NULL) == 0) {
        return xr_ok(xr_nil());
    }
#else
    if (utimes(p, NULL) == 0) {
        return xr_ok(xr_nil());
    }
#endif
    FILE *file = fopen(p, "ab");
    if (file == NULL) {
        return xr_fs_fail("touch", p);
    }
    fclose(file);
    return xr_ok(xr_nil());
}

#ifndef _WIN32
static int xr_fs_make_dir(const char *path) {
    struct stat info;
    if (stat(path, &info) == 0) {
        return S_ISDIR(info.st_mode) ? 0 : -1;
    }
    if (mkdir(path, 0777) == 0 || errno == EEXIST) {
        return 0;
    }
    return -1;
}

static int xr_fs_make_dirs(const char *path) {
    if (path[0] == '\0') {
        return -1;
    }
    char buffer[4096];
    size_t len = strlen(path);
    if (len >= sizeof(buffer)) {
        return -1;
    }
    memcpy(buffer, path, len + 1);
    for (size_t i = 1; i < len; i += 1) {
        if (buffer[i] == '/') {
            buffer[i] = '\0';
            if (xr_fs_make_dir(buffer) != 0) {
                return -1;
            }
            buffer[i] = '/';
        }
    }
    return xr_fs_make_dir(buffer);
}

static int xr_fs_remove_tree(const char *path) {
    struct stat info;
    if (lstat(path, &info) != 0) {
        return -1;
    }
    if (!S_ISDIR(info.st_mode) || S_ISLNK(info.st_mode)) {
        return unlink(path);
    }
    DIR *dir = opendir(path);
    if (dir == NULL) {
        return -1;
    }
    struct dirent *entry;
    while ((entry = readdir(dir)) != NULL) {
        if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) {
            continue;
        }
        size_t size = strlen(path) + strlen(entry->d_name) + 2;
        char *child = (char *)xr_alloc(size);
        snprintf(child, size, "%s/%s", path, entry->d_name);
        if (xr_fs_remove_tree(child) != 0) {
            closedir(dir);
            return -1;
        }
    }
    closedir(dir);
    return rmdir(path);
}
#endif

XVal xr_fs_mkdir(XVal path) {
    const char *p = xr_fs_path(path);
    if (p == NULL || p[0] == '\0') {
        return xr_err("std::fs::mkdir expects a non-empty string path");
    }
#ifdef _WIN32
    // `_mkdir` is not recursive; create each missing component.
    char buffer[4096];
    size_t len = strlen(p);
    if (len >= sizeof(buffer)) {
        return xr_fs_fail("create", p);
    }
    memcpy(buffer, p, len + 1);
    for (size_t i = 1; i < len; i += 1) {
        if (buffer[i] == '\\' || buffer[i] == '/') {
            char saved = buffer[i];
            buffer[i] = '\0';
            _mkdir(buffer);
            buffer[i] = saved;
        }
    }
    if (_mkdir(buffer) != 0 && errno != EEXIST) {
        return xr_fs_fail("create", p);
    }
    return xr_ok(xr_nil());
#else
    if (xr_fs_make_dirs(p) != 0) {
        return xr_fs_fail("create", p);
    }
    return xr_ok(xr_nil());
#endif
}

XVal xr_fs_remove_dir(XVal path) {
    const char *p = xr_fs_path(path);
    if (p == NULL) {
        return xr_err("std::fs::remove_dir expects a string path");
    }
#ifdef _WIN32
    if (_rmdir(p) != 0) {
        return xr_fs_fail("remove", p);
    }
    return xr_ok(xr_nil());
#else
    if (xr_fs_remove_tree(p) != 0) {
        return xr_fs_fail("remove", p);
    }
    return xr_ok(xr_nil());
#endif
}


XVal xr_fs_read_dir(XVal path) {
    const char *p = xr_fs_path(path);
    if (p == NULL) {
        return xr_err("std::fs::read_dir expects a string path");
    }
    XrNames names = {NULL, 0, 0};
    char error[512];
    error[0] = '\0';
    if (!xr_read_names(p, &names, error, sizeof(error))) {
        return xr_err(error[0] == '\0' ? "cannot read the directory" : error);
    }
    XVal out = xr_list_new();
    for (long long i = 0; i < names.len; i += 1) {
        xr_list_push(out, xr_str(names.items[i]));
    }
    return xr_ok(out);
}

XVal xr_fs_realpath(XVal path) {
    const char *p = xr_fs_path(path);
    if (p == NULL) {
        return xr_err("std::fs::realpath expects a string path");
    }
#ifdef _WIN32
    char buffer[4096];
    if (_fullpath(buffer, p, sizeof(buffer)) == NULL) {
        return xr_fs_fail("resolve", p);
    }
    return xr_ok(xr_str(buffer));
#else
    char buffer[4096];
    if (realpath(p, buffer) == NULL) {
        return xr_fs_fail("resolve", p);
    }
    return xr_ok(xr_str(buffer));
#endif
}

static const char *xr_fs_temp_root(void) {
#ifdef _WIN32
    static char buffer[MAX_PATH + 1];
    if (GetTempPathA(MAX_PATH + 1, buffer) == 0) {
        return ".";
    }
    return buffer;
#else
    const char *tmp = getenv("TMPDIR");
    if (tmp == NULL || tmp[0] == '\0') {
        tmp = "/tmp";
    }
    return tmp;
#endif
}

static XVal xr_fs_temp(const char *dir, XVal prefix, int want_dir) {
    const char *head = "";
    if (prefix.tag == XT_STR) {
        head = xr_as_str(prefix)->data;
    }
    static unsigned long long counter = 0;
    for (int attempt = 0; attempt < 4096; attempt += 1) {
        counter += 1;
        char buffer[4096];
        snprintf(buffer, sizeof(buffer), "%s/%s%llu-%llu", dir, head,
                 (unsigned long long)XR_GETPID(), (unsigned long long)counter);
#ifdef _WIN32
        if (want_dir) {
            if (_mkdir(buffer) == 0) {
                return xr_ok(xr_str(buffer));
            }
        } else {
            int fd = _open(buffer, _O_CREAT | _O_EXCL | _O_WRONLY | _O_BINARY,
                           _S_IREAD | _S_IWRITE);
            if (fd >= 0) {
                _close(fd);
                return xr_ok(xr_str(buffer));
            }
        }
#else
        if (want_dir) {
            if (mkdir(buffer, 0777) == 0) {
                return xr_ok(xr_str(buffer));
            }
        } else {
            int fd = open(buffer, O_CREAT | O_EXCL | O_WRONLY, 0600);
            if (fd >= 0) {
                close(fd);
                return xr_ok(xr_str(buffer));
            }
        }
#endif
    }
    return xr_fs_fail("create a temporary entry in", dir);
}

XVal xr_fs_temp_file(XVal prefix) {
    return xr_fs_temp(xr_fs_temp_root(), prefix, 0);
}

XVal xr_fs_temp_dir(XVal prefix) {
    return xr_fs_temp(xr_fs_temp_root(), prefix, 1);
}

XVal xr_fs_temp_file_in(XVal dir, XVal prefix) {
    const char *p = xr_fs_path(dir);
    if (p == NULL) {
        return xr_err("std::fs::temp_file_in expects a string directory");
    }
    return xr_fs_temp(p, prefix, 0);
}

XVal xr_fs_temp_dir_in(XVal dir, XVal prefix) {
    const char *p = xr_fs_path(dir);
    if (p == NULL) {
        return xr_err("std::fs::temp_dir_in expects a string directory");
    }
    return xr_fs_temp(p, prefix, 1);
}

