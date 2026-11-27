// ── std::process ─────────────────────────────────────────────────────────
//
// Process-level control (`args`, `exit`), the one-shot captures `run`/`exec`,
// the kept handle `launch`, and the `Command`/`Child` types of
// `std-spec/modules/process.md`. Both streams are captured separately, input
// is piped when supplied, and a millisecond budget bounds the child's life.

#define XR_PROCESS_MAX_CAPTURE (1LL << 24)

typedef struct {
    int shell;
    XStr *program;
    XList *args;
    XVal input;
    XVal cwd;
    XVal env;
    long long timeout_ms;
} XrProcessSpec;

typedef struct XrChild {
    int state; // 1 running, 2 spent
    long long pid;
    long long started_ms;
    long long timeout_ms;
    char *display;
#ifdef _WIN32
    HANDLE process;
    HANDLE out_read;
    HANDLE err_read;
    HANDLE in_write;
#else
    pid_t child;
    int out_fd;
    int err_fd;
    int in_fd;
    int status;
    int reaped;
#endif
} XrChild;

static XrChild **xr_child_table = NULL;
static long long xr_child_count = 0;
static long long xr_child_cap = 0;
static long long xr_child_next_id = 1;

#ifndef _WIN32
static int xr_process_sigpipe_ready = 0;
static void xr_process_ignore_sigpipe(void) {
    if (!xr_process_sigpipe_ready) {
        signal(SIGPIPE, SIG_IGN);
        xr_process_sigpipe_ready = 1;
    }
}
#endif

static long long xr_process_now_ms(void) {
#ifdef _WIN32
    return (long long)GetTickCount64();
#else
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (long long)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
#endif
}

static XVal xr_process_result_value(XVal out, XVal err, long long code) {
    const char *names[3];
    XVal values[3];
    names[0] = "stdout";
    names[1] = "stderr";
    names[2] = "code";
    values[0] = out;
    values[1] = err;
    values[2] = xr_int(code);
    return xr_ok(xr_struct("RunResult", 3, names, values));
}

static XVal xr_process_child_object(long long id) {
    const char *names[1];
    XVal values[1];
    names[0] = "id";
    values[0] = xr_int(id);
    return xr_struct("Child", 1, names, values);
}

static XrChild *xr_process_child_get(XVal value) {
    if (value.tag != XT_OBJ || strcmp(xr_as_obj(value)->type, "Child") != 0) {
        xr_fail("expected a `Child` handle");
    }
    XVal id = xr_get_field(value, "id");
    if (id.tag != XT_INT || id.i < 1 || id.i >= xr_child_count) {
        xr_fail("invalid `Child` handle");
    }
    return xr_child_table[id.i];
}

static long long xr_process_child_add(XrChild *child) {
    if (xr_child_count == xr_child_cap) {
        long long next = xr_child_cap == 0 ? 8 : xr_child_cap * 2;
        XrChild **table = (XrChild **)xr_alloc(sizeof(XrChild *) * (size_t)next);
        if (xr_child_table != NULL && xr_child_count > 0) {
            memcpy(table, xr_child_table, sizeof(XrChild *) * (size_t)xr_child_count);
        }
        for (long long i = xr_child_count; i < next; i += 1) {
            table[i] = NULL;
        }
        xr_child_table = table;
        xr_child_cap = next;
    }
    long long id = xr_child_next_id;
    xr_child_next_id += 1;
    xr_child_table[id] = child;
    xr_child_count = id + 1;
    return id;
}

static XVal xr_process_command_object(int shell, XVal program, XVal args, XVal input, XVal cwd,
                                      XVal env, XVal timeout) {
    const char *names[7];
    XVal values[7];
    names[0] = "kind";
    names[1] = "program";
    names[2] = "args";
    names[3] = "input";
    names[4] = "cwd";
    names[5] = "env";
    names[6] = "timeout";
    values[0] = xr_str(shell ? "shell" : "direct");
    values[1] = program;
    values[2] = args;
    values[3] = input;
    values[4] = cwd;
    values[5] = env;
    values[6] = timeout;
    return xr_struct("Command", 7, names, values);
}

static XVal xr_process_command_shell(XVal command) {
    if (command.tag != XT_STR) {
        return xr_nil();
    }
    XVal args = xr_list_new();
    return xr_process_command_object(1, command, args, xr_nil(), xr_nil(), xr_nil(), xr_nil());
}

static XVal xr_process_command_direct(XVal program, XVal args) {
    if (program.tag != XT_STR || args.tag != XT_LIST) {
        return xr_nil();
    }
    return xr_process_command_object(0, program, args, xr_nil(), xr_nil(), xr_nil(), xr_nil());
}

static XVal xr_process_command_clone_with(XVal command, const char *field, XVal value) {
    XVal kind = xr_get_field(command, "kind");
    XVal program = xr_get_field(command, "program");
    XVal args = xr_get_field(command, "args");
    XVal input = xr_get_field(command, "input");
    XVal cwd = xr_get_field(command, "cwd");
    XVal env = xr_get_field(command, "env");
    XVal timeout = xr_get_field(command, "timeout");
    if (strcmp(field, "input") == 0) {
        input = value;
    } else if (strcmp(field, "cwd") == 0) {
        cwd = value;
    } else if (strcmp(field, "env") == 0) {
        env = value;
    } else if (strcmp(field, "timeout") == 0) {
        timeout = value;
    }
    int shell = kind.tag == XT_STR && strcmp(xr_as_str(kind)->data, "shell") == 0;
    return xr_process_command_object(shell, program, args, input, cwd, env, timeout);
}

static XVal xr_process_command_env(XVal command, XVal name, XVal value) {
    XVal old = xr_get_field(command, "env");
    XVal env = xr_map_new();
    if (old.tag == XT_OBJ) {
        XObj *object = xr_as_obj(old);
        for (long long i = 0; i < object->nfields; i += 1) {
            xr_map_set(env, xr_str(object->fields[i].name), object->fields[i].value);
        }
    }
    xr_map_set(env, name, value);
    return xr_process_command_clone_with(command, "env", env);
}

static void xr_process_spec_from_command(XVal command, XrProcessSpec *spec) {
    XVal kind = xr_get_field(command, "kind");
    XVal program = xr_get_field(command, "program");
    XVal args = xr_get_field(command, "args");
    spec->shell = kind.tag == XT_STR && strcmp(xr_as_str(kind)->data, "shell") == 0;
    spec->program = program.tag == XT_STR ? xr_as_str(program) : NULL;
    spec->args = args.tag == XT_LIST ? xr_as_list(args) : NULL;
    spec->input = xr_get_field(command, "input");
    spec->cwd = xr_get_field(command, "cwd");
    spec->env = xr_get_field(command, "env");
    XVal timeout = xr_get_field(command, "timeout");
    spec->timeout_ms = timeout.tag == XT_INT ? timeout.i : -1;
}

static void xr_process_spec_shorthand(XrProcessSpec *spec, int shell, XVal program, XVal args,
                                      XVal input, XVal timeout) {
    spec->shell = shell;
    spec->program = program.tag == XT_STR ? xr_as_str(program) : NULL;
    spec->args = args.tag == XT_LIST ? xr_as_list(args) : NULL;
    spec->input = input;
    spec->cwd = xr_nil();
    spec->env = xr_nil();
    spec->timeout_ms = timeout.tag == XT_INT ? timeout.i : -1;
}

static const char *xr_process_display(XrProcessSpec *spec) {
    if (spec->program == NULL) {
        return "";
    }
    return spec->program->data;
}

static XVal xr_process_error_command(const char *message, const char *display) {
    char buffer[512];
    snprintf(buffer, sizeof(buffer), "%s `%s`", message, display);
    return xr_err(buffer);
}

#ifdef _WIN32

static void xr_process_append_arg(char **buf, long long *len, long long *cap, const char *text) {
    long long need = (long long)strlen(text) * 2 + 4;
    if (*len + need > *cap) {
        long long next = *cap == 0 ? 256 : *cap;
        while (next < *len + need) {
            next *= 2;
        }
        char *grown = (char *)xr_alloc((size_t)next);
        if (*len > 0) {
            memcpy(grown, *buf, (size_t)*len);
        }
        *buf = grown;
        *cap = next;
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

static char *xr_process_command_line(XrProcessSpec *spec) {
    char *line = NULL;
    long long len = 0;
    long long cap = 0;
    if (spec->shell) {
        // `cmd /C` reconstructs its command from the raw tail; quoting the
        // executable or the switch confuses its quote stripping.
        const char *display = xr_process_display(spec);
        size_t n = strlen("cmd.exe /C ") + strlen(display) + 1;
        char *line = (char *)xr_alloc(n);
        snprintf(line, n, "cmd.exe /C %s", display);
        return line;
    }
    xr_process_append_arg(&line, &len, &cap, xr_process_display(spec));
    if (spec->args != NULL) {
        for (long long i = 0; i < spec->args->len; i += 1) {
            if (spec->args->items[i].tag != XT_STR) {
                return NULL;
            }
            line[len++] = ' ';
            line[len] = '\0';
            xr_process_append_arg(&line, &len, &cap, xr_as_str(spec->args->items[i])->data);
        }
    }
    return line;
}

static char *xr_process_environment(XrProcessSpec *spec) {
    if (spec->env.tag != XT_OBJ) {
        return NULL;
    }
    XObj *overlay = xr_as_obj(spec->env);
    char *block = NULL;
    long long len = 0;
    long long cap = 0;
    LPCH inherited = GetEnvironmentStringsA();
    if (inherited != NULL) {
        const char *at = inherited;
        while (*at != '\0') {
            const char *entry = at;
            const char *eq = strchr(entry, '=');
            int overridden = 0;
            if (eq != NULL) {
                for (long long i = 0; i < overlay->nfields; i += 1) {
                    const char *name = overlay->fields[i].name;
                    size_t n = strlen(name);
                    if ((size_t)(eq - entry) == n && strncmp(entry, name, n) == 0) {
                        overridden = 1;
                        break;
                    }
                }
            }
            if (!overridden) {
                size_t n = strlen(entry) + 1;
                if (len + (long long)n + 1 > cap) {
                    long long next = cap == 0 ? 256 : cap;
                    while (next < len + (long long)n + 1) {
                        next *= 2;
                    }
                    char *grown = (char *)xr_alloc((size_t)next);
                    if (len > 0) {
                        memcpy(grown, block, (size_t)len);
                    }
                    block = grown;
                    cap = next;
                }
                memcpy(block + len, entry, n);
                len += (long long)n;
            }
            at += strlen(at) + 1;
        }
        FreeEnvironmentStringsA(inherited);
    }
    for (long long i = 0; i < overlay->nfields; i += 1) {
        const char *name = overlay->fields[i].name;
        XVal value = overlay->fields[i].value;
        const char *text = value.tag == XT_STR ? xr_as_str(value)->data : "";
        size_t n = strlen(name) + 1 + strlen(text) + 1;
        if (len + (long long)n + 1 > cap) {
            long long next = cap == 0 ? 256 : cap;
            while (next < len + (long long)n + 1) {
                next *= 2;
            }
            char *grown = (char *)xr_alloc((size_t)next);
            if (len > 0) {
                memcpy(grown, block, (size_t)len);
            }
            block = grown;
            cap = next;
        }
        snprintf(block + len, (size_t)(cap - len), "%s=%s", name, text);
        len += (long long)strlen(block + len) + 1;
    }
    if (block == NULL) {
        block = (char *)xr_alloc(1);
    }
    block[len] = '\0';
    return block;
}

static XrChild *xr_process_spawn(XrProcessSpec *spec, char *error, size_t error_cap) {
    const char *display = xr_process_display(spec);
    char *line = xr_process_command_line(spec);
    if (line == NULL) {
        snprintf(error, error_cap, "cannot run `%s`: arguments must be strings", display);
        return NULL;
    }
    SECURITY_ATTRIBUTES sa;
    sa.nLength = sizeof(sa);
    sa.lpSecurityDescriptor = NULL;
    sa.bInheritHandle = TRUE;
    HANDLE out_read = NULL;
    HANDLE out_write = NULL;
    HANDLE err_read = NULL;
    HANDLE err_write = NULL;
    HANDLE in_read = NULL;
    HANDLE in_write = NULL;
    if (!CreatePipe(&out_read, &out_write, &sa, 0) || !CreatePipe(&err_read, &err_write, &sa, 0)) {
        snprintf(error, error_cap, "cannot run `%s`: cannot create a pipe", display);
        return NULL;
    }
    SetHandleInformation(out_read, HANDLE_FLAG_INHERIT, 0);
    SetHandleInformation(err_read, HANDLE_FLAG_INHERIT, 0);
    if (spec->input.tag == XT_STR) {
        if (!CreatePipe(&in_read, &in_write, &sa, 0)) {
            CloseHandle(out_read);
            CloseHandle(out_write);
            CloseHandle(err_read);
            CloseHandle(err_write);
            snprintf(error, error_cap, "cannot run `%s`: cannot create a pipe", display);
            return NULL;
        }
        SetHandleInformation(in_write, HANDLE_FLAG_INHERIT, 0);
    }
    STARTUPINFOA si;
    ZeroMemory(&si, sizeof(si));
    si.cb = sizeof(si);
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdOutput = out_write;
    si.hStdError = err_write;
    si.hStdInput = in_read != NULL ? in_read : GetStdHandle(STD_INPUT_HANDLE);
    PROCESS_INFORMATION pi;
    ZeroMemory(&pi, sizeof(pi));
    char *env = xr_process_environment(spec);
    const char *cwd = spec->cwd.tag == XT_STR ? xr_as_str(spec->cwd)->data : NULL;
    BOOL started = CreateProcessA(NULL, line, NULL, NULL, TRUE, CREATE_NO_WINDOW, env, cwd, &si,
                                  &pi);
    CloseHandle(out_write);
    CloseHandle(err_write);
    if (in_read != NULL) {
        CloseHandle(in_read);
    }
    if (!started) {
        CloseHandle(out_read);
        CloseHandle(err_read);
        if (in_write != NULL) {
            CloseHandle(in_write);
        }
        snprintf(error, error_cap, "cannot run `%s`: error %lu", display,
                 (unsigned long)GetLastError());
        return NULL;
    }
    if (spec->input.tag == XT_STR) {
        XStr *text = xr_as_str(spec->input);
        DWORD written = 0;
        WriteFile(in_write, text->data, (DWORD)text->bytes, &written, NULL);
        CloseHandle(in_write);
    }
    XrChild *child = (XrChild *)xr_alloc(sizeof(XrChild));
    child->state = 1;
    child->pid = (long long)pi.dwProcessId;
    child->started_ms = xr_process_now_ms();
    child->timeout_ms = spec->timeout_ms;
    child->display = xr_copy_bytes(display, (long long)strlen(display));
    child->process = pi.hProcess;
    child->out_read = out_read;
    child->err_read = err_read;
    child->in_write = NULL;
    CloseHandle(pi.hThread);
    return child;
}

static char *xr_process_read_handle(HANDLE handle, long long *bytes, int *over) {
    long long cap = 4096;
    long long len = 0;
    char *data = (char *)xr_alloc((size_t)cap);
    DWORD got = 0;
    *over = 0;
    while (ReadFile(handle, data + len, (DWORD)(cap - len - 1), &got, NULL) && got > 0) {
        len += (long long)got;
        if (len > XR_PROCESS_MAX_CAPTURE) {
            *over = 1;
            break;
        }
        if (len + 1 >= cap) {
            long long next = cap * 2;
            char *grown = (char *)xr_alloc((size_t)next);
            memcpy(grown, data, (size_t)len);
            data = grown;
            cap = next;
        }
    }
    data[len] = '\0';
    *bytes = len;
    return data;
}

static int xr_process_wait_child(XrChild *child, XVal *result, char *error, size_t error_cap) {
    if (child->state == 2) {
        snprintf(error, error_cap, "`%s` child handle is spent", child->display);
        return 0;
    }
    child->state = 2;
    long long out_bytes = 0;
    long long err_bytes = 0;
    int out_over = 0;
    int err_over = 0;
    char *out = xr_process_read_handle(child->out_read, &out_bytes, &out_over);
    char *err = xr_process_read_handle(child->err_read, &err_bytes, &err_over);
    CloseHandle(child->out_read);
    CloseHandle(child->err_read);
    child->out_read = NULL;
    child->err_read = NULL;
    if (out_over || err_over) {
        TerminateProcess(child->process, 1);
        WaitForSingleObject(child->process, INFINITE);
        CloseHandle(child->process);
        child->process = NULL;
        snprintf(error, error_cap, "`%s` output exceeds %lld bytes per stream", child->display,
                 XR_PROCESS_MAX_CAPTURE);
        return 0;
    }
    DWORD wait_ms = INFINITE;
    if (child->timeout_ms >= 0) {
        long long remaining = child->started_ms + child->timeout_ms - xr_process_now_ms();
        wait_ms = remaining <= 0 ? 0 : (DWORD)remaining;
    }
    DWORD waited = WaitForSingleObject(child->process, wait_ms);
    if (waited == WAIT_TIMEOUT) {
        TerminateProcess(child->process, 1);
        WaitForSingleObject(child->process, INFINITE);
        DWORD ignored = 0;
        GetExitCodeProcess(child->process, &ignored);
        CloseHandle(child->process);
        child->process = NULL;
        snprintf(error, error_cap, "`%s` timed out after %lld ms", child->display,
                 child->timeout_ms);
        return 0;
    }
    DWORD code = 1;
    GetExitCodeProcess(child->process, &code);
    CloseHandle(child->process);
    child->process = NULL;
    *result = xr_process_result_value(xr_net_lossy_string((const unsigned char *)out, out_bytes),
                                      xr_net_lossy_string((const unsigned char *)err, err_bytes),
                                      (long long)code);
    return 1;
}

static int xr_process_kill_child(XrChild *child, char *error, size_t error_cap) {
    if (child->state == 2) {
        snprintf(error, error_cap, "`%s` child handle is spent", child->display);
        return 0;
    }
    DWORD waited = WaitForSingleObject(child->process, 0);
    if (waited == WAIT_OBJECT_0) {
        snprintf(error, error_cap, "`%s` child has already exited", child->display);
        return 0;
    }
    if (!TerminateProcess(child->process, 1)) {
        snprintf(error, error_cap, "cannot kill `%s`", child->display);
        return 0;
    }
    return 1;
}

#else

static char **xr_process_argv(XrProcessSpec *spec) {
    long long count = spec->args == NULL ? 0 : spec->args->len;
    char **argv = (char **)xr_alloc(sizeof(char *) * (size_t)(count + 2));
    argv[0] = (char *)xr_process_display(spec);
    for (long long i = 0; i < count; i += 1) {
        if (spec->args->items[i].tag != XT_STR) {
            return NULL;
        }
        argv[i + 1] = xr_as_str(spec->args->items[i])->data;
    }
    return argv;
}

static int xr_process_spawn(XrProcessSpec *spec, XrChild **out, char *error, size_t error_cap) {
    xr_process_ignore_sigpipe();
    const char *display = xr_process_display(spec);
    int out_pipe[2];
    int err_pipe[2];
    int in_pipe[2];
    int exec_pipe[2];
    if (pipe(out_pipe) != 0 || pipe(err_pipe) != 0 || pipe(exec_pipe) != 0) {
        snprintf(error, error_cap, "cannot run `%s`: cannot create a pipe", display);
        return 0;
    }
    fcntl(exec_pipe[1], F_SETFD, FD_CLOEXEC);
    int has_input = spec->input.tag == XT_STR;
    if (has_input && pipe(in_pipe) != 0) {
        close(out_pipe[0]);
        close(out_pipe[1]);
        close(err_pipe[0]);
        close(err_pipe[1]);
        close(exec_pipe[0]);
        close(exec_pipe[1]);
        snprintf(error, error_cap, "cannot run `%s`: cannot create a pipe", display);
        return 0;
    }
    pid_t pid = fork();
    if (pid < 0) {
        close(out_pipe[0]);
        close(out_pipe[1]);
        close(err_pipe[0]);
        close(err_pipe[1]);
        close(exec_pipe[0]);
        close(exec_pipe[1]);
        if (has_input) {
            close(in_pipe[0]);
            close(in_pipe[1]);
        }
        snprintf(error, error_cap, "cannot run `%s`: cannot fork", display);
        return 0;
    }
    if (pid == 0) {
        close(exec_pipe[0]);
        if (has_input) {
            dup2(in_pipe[0], 0);
        }
        dup2(out_pipe[1], 1);
        dup2(err_pipe[1], 2);
        close(out_pipe[0]);
        close(out_pipe[1]);
        close(err_pipe[0]);
        close(err_pipe[1]);
        if (has_input) {
            close(in_pipe[0]);
            close(in_pipe[1]);
        }
        if (spec->cwd.tag == XT_STR) {
            if (chdir(xr_as_str(spec->cwd)->data) != 0) {
                int code = errno;
                write(exec_pipe[1], &code, sizeof(code));
                _exit(127);
            }
        }
        if (spec->env.tag == XT_OBJ) {
            XObj *overlay = xr_as_obj(spec->env);
            for (long long i = 0; i < overlay->nfields; i += 1) {
                const char *value = overlay->fields[i].value.tag == XT_STR
                                        ? xr_as_str(overlay->fields[i].value)->data
                                        : "";
                setenv(overlay->fields[i].name, value, 1);
            }
        }
        if (spec->shell) {
            execl("/bin/sh", "sh", "-c", display, (char *)NULL);
        } else {
            char **argv = xr_process_argv(spec);
            if (argv != NULL) {
                execvp(argv[0], argv);
            }
        }
        {
            int code = errno;
            write(exec_pipe[1], &code, sizeof(code));
        }
        _exit(127);
    }
    close(out_pipe[1]);
    close(err_pipe[1]);
    close(exec_pipe[1]);
    int exec_error = 0;
    if (read(exec_pipe[0], &exec_error, sizeof(exec_error)) > 0) {
        close(out_pipe[0]);
        close(err_pipe[0]);
        if (has_input) {
            close(in_pipe[0]);
            close(in_pipe[1]);
        }
        close(exec_pipe[0]);
        int status = 0;
        waitpid(pid, &status, 0);
        snprintf(error, error_cap, "cannot run `%s`: error %d", display, exec_error);
        return 0;
    }
    close(exec_pipe[0]);
    if (has_input) {
        close(in_pipe[0]);
        XStr *text = xr_as_str(spec->input);
        long long written = 0;
        while (written < text->bytes) {
            ssize_t n = write(in_pipe[1], text->data + written,
                              (size_t)(text->bytes - written));
            if (n <= 0) {
                break;
            }
            written += (long long)n;
        }
        close(in_pipe[1]);
    }
    XrChild *child = (XrChild *)xr_alloc(sizeof(XrChild));
    child->state = 1;
    child->pid = (long long)pid;
    child->child = pid;
    child->started_ms = xr_process_now_ms();
    child->timeout_ms = spec->timeout_ms;
    child->display = xr_copy_bytes(display, (long long)strlen(display));
    child->out_fd = out_pipe[0];
    child->err_fd = err_pipe[0];
    child->in_fd = -1;
    child->reaped = 0;
    *out = child;
    return 1;
}

static void xr_process_collect(int fd, char **data, long long *len, long long *cap, int *over,
                               int *open) {
    if (!*open) {
        return;
    }
    char chunk[4096];
    ssize_t got = read(fd, chunk, sizeof(chunk));
    if (got > 0) {
        if (*len + got > XR_PROCESS_MAX_CAPTURE) {
            *over = 1;
            *open = 0;
            return;
        }
        if (*len + got + 1 > *cap) {
            long long next = *cap == 0 ? 8192 : *cap;
            while (next < *len + got + 1) {
                next *= 2;
            }
            char *grown = (char *)xr_alloc((size_t)next);
            if (*len > 0) {
                memcpy(grown, *data, (size_t)*len);
            }
            *data = grown;
            *cap = next;
        }
        memcpy(*data + *len, chunk, (size_t)got);
        *len += (long long)got;
        return;
    }
    if (got == 0) {
        *open = 0;
        return;
    }
    if (errno == EINTR || errno == EAGAIN || errno == EWOULDBLOCK) {
        return;
    }
    *open = 0;
}

static int xr_process_wait_child(XrChild *child, XVal *result, char *error, size_t error_cap) {
    if (child->state == 2) {
        snprintf(error, error_cap, "`%s` child handle is spent", child->display);
        return 0;
    }
    child->state = 2;
    char *out = NULL;
    long long out_len = 0;
    long long out_cap = 0;
    char *err = NULL;
    long long err_len = 0;
    long long err_cap = 0;
    int out_open = 1;
    int err_open = 1;
    int over = 0;
    long long deadline = child->timeout_ms >= 0 ? child->started_ms + child->timeout_ms : -1;
    int timed_out = 0;
    while (out_open || err_open) {
        if (deadline >= 0) {
            long long remaining = deadline - xr_process_now_ms();
            if (remaining <= 0) {
                timed_out = 1;
                break;
            }
        }
        struct pollfd fds[2];
        int count = 0;
        int out_index = -1;
        int err_index = -1;
        if (out_open) {
            out_index = count;
            fds[count].fd = child->out_fd;
            fds[count].events = POLLIN;
            fds[count].revents = 0;
            count += 1;
        }
        if (err_open) {
            err_index = count;
            fds[count].fd = child->err_fd;
            fds[count].events = POLLIN;
            fds[count].revents = 0;
            count += 1;
        }
        int wait = deadline >= 0 ? (int)(deadline - xr_process_now_ms()) : -1;
        if (wait < 0) {
            wait = -1;
        }
        int ready = poll(fds, (nfds_t)count, wait);
        if (ready < 0) {
            if (errno == EINTR) {
                continue;
            }
            over = 1;
            break;
        }
        if (ready == 0) {
            timed_out = 1;
            break;
        }
        if (out_index >= 0 && (fds[out_index].revents & (POLLIN | POLLHUP | POLLERR)) != 0) {
            xr_process_collect(child->out_fd, &out, &out_len, &out_cap, &over, &out_open);
        }
        if (err_index >= 0 && (fds[err_index].revents & (POLLIN | POLLHUP | POLLERR)) != 0) {
            xr_process_collect(child->err_fd, &err, &err_len, &err_cap, &over, &err_open);
        }
        if (over) {
            break;
        }
    }
    if (child->out_fd >= 0) {
        close(child->out_fd);
        child->out_fd = -1;
    }
    if (child->err_fd >= 0) {
        close(child->err_fd);
        child->err_fd = -1;
    }
    if (over || timed_out) {
        kill(child->child, SIGKILL);
        int status = 0;
        waitpid(child->child, &status, 0);
        child->reaped = 1;
        if (timed_out) {
            snprintf(error, error_cap, "`%s` timed out after %lld ms", child->display,
                     child->timeout_ms);
        } else {
            snprintf(error, error_cap, "`%s` output exceeds %lld bytes per stream", child->display,
                     XR_PROCESS_MAX_CAPTURE);
        }
        return 0;
    }
    int status = 0;
    if (child->reaped) {
        status = child->status;
    } else if (waitpid(child->child, &status, 0) < 0) {
        snprintf(error, error_cap, "cannot wait for `%s`", child->display);
        return 0;
    } else {
        child->reaped = 1;
        child->status = status;
    }
    long long code = WIFEXITED(status) ? (long long)WEXITSTATUS(status) : -1;
    *result = xr_process_result_value(xr_net_lossy_string((const unsigned char *)out, out_len),
                                      xr_net_lossy_string((const unsigned char *)err, err_len),
                                      code);
    return 1;
}

static int xr_process_kill_child(XrChild *child, char *error, size_t error_cap) {
    if (child->state == 2) {
        snprintf(error, error_cap, "`%s` child handle is spent", child->display);
        return 0;
    }
    int status = 0;
    pid_t done = waitpid(child->child, &status, WNOHANG);
    if (done == child->child) {
        child->reaped = 1;
        child->status = status;
        snprintf(error, error_cap, "`%s` child has already exited", child->display);
        return 0;
    }
    if (kill(child->child, SIGKILL) != 0) {
        snprintf(error, error_cap, "cannot kill `%s`", child->display);
        return 0;
    }
    return 1;
}

#endif

static XVal xr_process_run_spec(XrProcessSpec *spec) {
#ifdef _WIN32
    char error[512];
    XrChild *child = xr_process_spawn(spec, error, sizeof(error));
    if (child == NULL) {
        return xr_err(error);
    }
#else
    char error[512];
    XrChild *child = NULL;
    if (!xr_process_spawn(spec, &child, error, sizeof(error))) {
        return xr_err(error);
    }
#endif
    XVal result = xr_nil();
    char wait_error[512];
    int ok = xr_process_wait_child(child, &result, wait_error, sizeof(wait_error));
    if (!ok) {
        return xr_err(wait_error);
    }
    return result;
}

static XVal xr_process_launch_spec(XrProcessSpec *spec) {
#ifdef _WIN32
    char error[512];
    XrChild *child = xr_process_spawn(spec, error, sizeof(error));
    if (child == NULL) {
        return xr_err(error);
    }
#else
    char error[512];
    XrChild *child = NULL;
    if (!xr_process_spawn(spec, &child, error, sizeof(error))) {
        return xr_err(error);
    }
#endif
    long long id = xr_process_child_add(child);
    return xr_ok(xr_process_child_object(id));
}

XVal xr_process_run(XVal command, XVal input, XVal timeout) {
    if (command.tag != XT_STR) {
        return xr_err("std::process::run expects a command string");
    }
    XrProcessSpec spec;
    xr_process_spec_shorthand(&spec, 1, command, xr_nil(), input, timeout);
    return xr_process_run_spec(&spec);
}

XVal xr_process_exec(XVal program, XVal args, XVal input, XVal timeout) {
    if (program.tag != XT_STR || args.tag != XT_LIST) {
        return xr_err("std::process::exec expects a program and an argument list");
    }
    XrProcessSpec spec;
    xr_process_spec_shorthand(&spec, 0, program, args, input, timeout);
    return xr_process_run_spec(&spec);
}

XVal xr_process_launch(XVal command, XVal input, XVal timeout) {
    if (command.tag != XT_STR) {
        return xr_err("std::process::launch expects a command string");
    }
    XrProcessSpec spec;
    xr_process_spec_shorthand(&spec, 1, command, xr_nil(), input, timeout);
    return xr_process_launch_spec(&spec);
}

XVal xr_process_command_method(XVal recv, const char *name, long long n, XVal *args) {
    if (strcmp(name, "input") == 0) {
        if (n != 1 || args[0].tag != XT_STR) {
            return xr_err("`Command::input` expects a string");
        }
        return xr_process_command_clone_with(recv, "input", args[0]);
    }
    if (strcmp(name, "cwd") == 0) {
        if (n != 1 || args[0].tag != XT_STR) {
            return xr_err("`Command::cwd` expects a string");
        }
        return xr_process_command_clone_with(recv, "cwd", args[0]);
    }
    if (strcmp(name, "env") == 0) {
        if (n != 2 || args[0].tag != XT_STR || args[1].tag != XT_STR) {
            return xr_err("`Command::env` expects a name and a value");
        }
        return xr_process_command_env(recv, args[0], args[1]);
    }
    if (strcmp(name, "timeout") == 0) {
        if (n != 1 || args[0].tag != XT_INT) {
            return xr_err("`Command::timeout` expects milliseconds");
        }
        return xr_process_command_clone_with(recv, "timeout", args[0]);
    }
    if (strcmp(name, "run") == 0) {
        if (n != 0) {
            return xr_err("`Command::run` expects no arguments");
        }
        XrProcessSpec spec;
        xr_process_spec_from_command(recv, &spec);
        return xr_process_run_spec(&spec);
    }
    if (strcmp(name, "launch") == 0) {
        if (n != 0) {
            return xr_err("`Command::launch` expects no arguments");
        }
        XrProcessSpec spec;
        xr_process_spec_from_command(recv, &spec);
        return xr_process_launch_spec(&spec);
    }
    return xr_err("unknown `Command` method");
}

XVal xr_process_child_method(XVal recv, const char *name, long long n, XVal *args) {
    XrChild *child = xr_process_child_get(recv);
    if (strcmp(name, "pid") == 0) {
        return xr_int(child->pid);
    }
    if (strcmp(name, "kill") == 0) {
        if (n != 0) {
            return xr_err("`Child::kill` expects no arguments");
        }
        char error[512];
        if (!xr_process_kill_child(child, error, sizeof(error))) {
            return xr_err(error);
        }
        return xr_ok(xr_nil());
    }
    if (strcmp(name, "wait") == 0) {
        if (n != 0) {
            return xr_err("`Child::wait` expects no arguments");
        }
        XVal result = xr_nil();
        char error[512];
        if (!xr_process_wait_child(child, &result, error, sizeof(error))) {
            return xr_err(error);
        }
        return result;
    }
    return xr_err("unknown `Child` method");
}
