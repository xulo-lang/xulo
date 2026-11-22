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
