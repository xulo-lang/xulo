// ── std::net ─────────────────────────────────────────────────────────────
//
// TCP over IPv4/IPv6 with opaque numeric handles (std-spec/modules/net.md).
// Every member validates its handle first, failures are `Result::Err`, end of
// stream is `Ok(null)`, and a caller-supplied timeout bounds the four blocking
// calls. The implementation is platform-split only at the socket calls.

#ifdef _WIN32
typedef SOCKET XrNetFd;
#define XR_NET_INVALID INVALID_SOCKET
#define xr_net_close_socket closesocket
#define XR_NET_LAST_ERROR WSAGetLastError()
#define XR_NET_IN_PROGRESS(e) ((e) == WSAEWOULDBLOCK)
#define XR_NET_INTR(e) ((e) == WSAEINTR)
#define XR_NET_SHUT_RDWR SD_BOTH
#define XR_NET_SHUT_WR SD_SEND
#else
typedef int XrNetFd;
#define XR_NET_INVALID (-1)
#define xr_net_close_socket close
#define XR_NET_LAST_ERROR errno
#define XR_NET_IN_PROGRESS(e) ((e) == EINPROGRESS || (e) == EWOULDBLOCK)
#define XR_NET_INTR(e) ((e) == EINTR)
#define XR_NET_SHUT_RDWR SHUT_RDWR
#define XR_NET_SHUT_WR SHUT_WR
#endif

typedef struct {
    int kind; // 0 = closed, 1 = listener, 2 = stream
    XrNetFd fd;
} XrNetEntry;

static XrNetEntry *xr_net_entries = NULL;
static long long xr_net_entries_len = 0;
static long long xr_net_next_handle = 1;
static int xr_net_initialized = 0;

static void xr_net_init(void) {
    if (xr_net_initialized) {
        return;
    }
#ifdef _WIN32
    WSADATA data;
    WSAStartup(MAKEWORD(2, 2), &data);
#endif
    xr_net_initialized = 1;
}

static void xr_net_grow(long long handle) {
    if (handle < xr_net_entries_len) {
        return;
    }
    long long next = xr_net_entries_len == 0 ? 16 : xr_net_entries_len;
    while (next <= handle) {
        next *= 2;
    }
    XrNetEntry *entries = (XrNetEntry *)xr_alloc(sizeof(XrNetEntry) * (size_t)next);
    if (xr_net_entries != NULL && xr_net_entries_len > 0) {
        memcpy(entries, xr_net_entries, sizeof(XrNetEntry) * (size_t)xr_net_entries_len);
    }
    for (long long i = xr_net_entries_len; i < next; i += 1) {
        entries[i].kind = 0;
        entries[i].fd = XR_NET_INVALID;
    }
    xr_net_entries = entries;
    xr_net_entries_len = next;
}

static long long xr_net_register(int kind, XrNetFd fd) {
    xr_net_init();
    long long handle = xr_net_next_handle;
    xr_net_next_handle += 1;
    xr_net_grow(handle);
    xr_net_entries[handle].kind = kind;
    xr_net_entries[handle].fd = fd;
    return handle;
}

static XrNetEntry *xr_net_entry(long long handle) {
    if (handle < 1 || handle >= xr_net_entries_len || xr_net_entries[handle].kind == 0) {
        return NULL;
    }
    return &xr_net_entries[handle];
}

static XVal xr_net_not_open(long long handle) {
    char message[96];
    snprintf(message, sizeof(message), "socket handle %lld is not open", handle);
    return xr_err(message);
}

static XVal xr_net_wrong_kind(long long handle, const char *actual, const char *wanted) {
    char message[128];
    snprintf(message, sizeof(message), "socket handle %lld is a %s, not a %s", handle, actual,
             wanted);
    return xr_err(message);
}

static int xr_net_set_nonblocking(XrNetFd fd, int on) {
#ifdef _WIN32
    u_long mode = on ? 1 : 0;
    return ioctlsocket(fd, FIONBIO, &mode) == 0;
#else
    int flags = fcntl(fd, F_GETFL, 0);
    if (flags < 0) {
        return 0;
    }
    int next = on ? (flags | O_NONBLOCK) : (flags & ~O_NONBLOCK);
    return fcntl(fd, F_SETFL, next) == 0;
#endif
}

static long long xr_net_now_ms(void) {
#ifdef _WIN32
    return (long long)GetTickCount64();
#else
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (long long)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
#endif
}

// 1 ready, 0 timed out, -1 error. `timeout_ms < 0` means wait without limit.
static int xr_net_wait(XrNetFd fd, int writing, long long timeout_ms) {
    fd_set set;
    FD_ZERO(&set);
    FD_SET(fd, &set);
    struct timeval tv;
    struct timeval *tvp = NULL;
    if (timeout_ms >= 0) {
        tv.tv_sec = (long)(timeout_ms / 1000);
        tv.tv_usec = (long)((timeout_ms % 1000) * 1000);
        tvp = &tv;
    }
#ifdef _WIN32
    int rc = select(0, writing ? NULL : &set, writing ? &set : NULL, NULL, tvp);
#else
    int rc = select(fd + 1, writing ? NULL : &set, writing ? &set : NULL, NULL, tvp);
#endif
    if (rc > 0) {
        return 1;
    }
    if (rc == 0) {
        return 0;
    }
    return -1;
}

static long long xr_net_timeout_ms(XVal value) {
    if (value.tag == XT_NIL) {
        return -1;
    }
    if (value.tag == XT_INT) {
        return value.i < 0 ? 0 : value.i;
    }
    if (value.tag == XT_FLOAT) {
        if (!isfinite(value.f)) {
            return 0;
        }
        long long amount = (long long)value.f;
        return amount < 0 ? 0 : amount;
    }
    return 0;
}

static int xr_net_split_addr(const char *addr, char *host, size_t host_cap, char *port,
                             size_t port_cap) {
    size_t n = strlen(addr);
    if (n == 0) {
        return 0;
    }
    if (addr[0] == '[') {
        const char *close = strchr(addr, ']');
        if (close == NULL || close[1] != ':') {
            return 0;
        }
        size_t host_len = (size_t)(close - addr - 1);
        if (host_len >= host_cap) {
            return 0;
        }
        memcpy(host, addr + 1, host_len);
        host[host_len] = '\0';
        const char *rest = close + 2;
        if (strlen(rest) >= port_cap) {
            return 0;
        }
        snprintf(port, port_cap, "%s", rest);
        return 1;
    }
    const char *colon = strrchr(addr, ':');
    if (colon == NULL) {
        return 0;
    }
    size_t host_len = (size_t)(colon - addr);
    if (host_len >= host_cap) {
        return 0;
    }
    memcpy(host, addr, host_len);
    host[host_len] = '\0';
    if (strlen(colon + 1) >= port_cap) {
        return 0;
    }
    snprintf(port, port_cap, "%s", colon + 1);
    return 1;
}

static int xr_net_resolve(const char *addr, int passive, struct addrinfo **result, char *error,
                          size_t error_cap) {
    char host[512];
    char port[32];
    if (!xr_net_split_addr(addr, host, sizeof(host), port, sizeof(port))) {
        snprintf(error, error_cap, "cannot resolve `%s`", addr);
        return 0;
    }
    struct addrinfo hints;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    hints.ai_protocol = IPPROTO_TCP;
    if (passive) {
        hints.ai_flags = AI_PASSIVE;
    }
    int rc = getaddrinfo(host[0] == '\0' ? NULL : host, port, &hints, result);
    if (rc != 0) {
#ifdef _WIN32
        snprintf(error, error_cap, "cannot resolve `%s`: %s", addr, gai_strerrorA(rc));
#else
        snprintf(error, error_cap, "cannot resolve `%s`: %s", addr, gai_strerror(rc));
#endif
        return 0;
    }
    return 1;
}

XVal xr_net_tcp_listen(XVal addr_value) {
    if (addr_value.tag != XT_STR) {
        return xr_err("std::net::tcp_listen expects a string address");
    }
    xr_net_init();
    char error[512];
    struct addrinfo *addresses = NULL;
    if (!xr_net_resolve(xr_as_str(addr_value)->data, 1, &addresses, error, sizeof(error))) {
        return xr_err(error);
    }
    XVal failure = xr_err("cannot listen on the requested address");
    for (struct addrinfo *ai = addresses; ai != NULL; ai = ai->ai_next) {
        XrNetFd fd = socket(ai->ai_family, ai->ai_socktype, ai->ai_protocol);
        if (fd == XR_NET_INVALID) {
            continue;
        }
        if (bind(fd, ai->ai_addr, (socklen_t)ai->ai_addrlen) == 0 &&
            listen(fd, SOMAXCONN) == 0) {
            freeaddrinfo(addresses);
            return xr_ok(xr_int(xr_net_register(1, fd)));
        }
        int code = XR_NET_LAST_ERROR;
        xr_net_close_socket(fd);
        char message[256];
        snprintf(message, sizeof(message), "cannot listen on `%s`: error %d",
                 xr_as_str(addr_value)->data, code);
        failure = xr_err(message);
    }
    freeaddrinfo(addresses);
    return failure;
}

static XVal xr_net_connect_address(struct addrinfo *ai, long long timeout_ms, XVal addr_value) {
    XrNetFd fd = socket(ai->ai_family, ai->ai_socktype, ai->ai_protocol);
    if (fd == XR_NET_INVALID) {
        return xr_err("cannot create a socket");
    }
    if (timeout_ms < 0) {
        if (connect(fd, ai->ai_addr, (socklen_t)ai->ai_addrlen) == 0) {
            return xr_ok(xr_int(xr_net_register(2, fd)));
        }
        int code = XR_NET_LAST_ERROR;
        xr_net_close_socket(fd);
        char message[256];
        snprintf(message, sizeof(message), "cannot connect to `%s`: error %d",
                 xr_as_str(addr_value)->data, code);
        return xr_err(message);
    }
    xr_net_set_nonblocking(fd, 1);
    int rc = connect(fd, ai->ai_addr, (socklen_t)ai->ai_addrlen);
    if (rc != 0) {
        int code = XR_NET_LAST_ERROR;
        if (!XR_NET_IN_PROGRESS(code)) {
            xr_net_close_socket(fd);
            char message[256];
            snprintf(message, sizeof(message), "cannot connect to `%s`: error %d",
                     xr_as_str(addr_value)->data, code);
            return xr_err(message);
        }
        int ready = xr_net_wait(fd, 1, timeout_ms);
        if (ready == 0) {
            xr_net_close_socket(fd);
            char message[128];
            snprintf(message, sizeof(message), "connect timed out after %lld ms", timeout_ms);
            return xr_err(message);
        }
        if (ready < 0) {
            int err = XR_NET_LAST_ERROR;
            xr_net_close_socket(fd);
            char message[256];
            snprintf(message, sizeof(message), "cannot connect to `%s`: error %d",
                     xr_as_str(addr_value)->data, err);
            return xr_err(message);
        }
        int so_error = 0;
        socklen_t so_len = sizeof(so_error);
        if (getsockopt(fd, SOL_SOCKET, SO_ERROR, (char *)&so_error, &so_len) != 0 ||
            so_error != 0) {
            if (so_error == 0) {
                so_error = XR_NET_LAST_ERROR;
            }
            xr_net_close_socket(fd);
            char message[256];
            snprintf(message, sizeof(message), "cannot connect to `%s`: error %d",
                     xr_as_str(addr_value)->data, so_error);
            return xr_err(message);
        }
    }
    xr_net_set_nonblocking(fd, 0);
    return xr_ok(xr_int(xr_net_register(2, fd)));
}

XVal xr_net_tcp_connect(XVal addr_value, XVal timeout_value) {
    if (addr_value.tag != XT_STR) {
        return xr_err("std::net::tcp_connect expects a string address");
    }
    xr_net_init();
    char error[512];
    struct addrinfo *addresses = NULL;
    if (!xr_net_resolve(xr_as_str(addr_value)->data, 0, &addresses, error, sizeof(error))) {
        return xr_err(error);
    }
    long long timeout_ms = xr_net_timeout_ms(timeout_value);
    XVal failure = xr_err("cannot connect to the requested address");
    for (struct addrinfo *ai = addresses; ai != NULL; ai = ai->ai_next) {
        failure = xr_net_connect_address(ai, timeout_ms, addr_value);
        if (xr_is_variant(failure, "Result::Ok")) {
            break;
        }
    }
    freeaddrinfo(addresses);
    return failure;
}

XVal xr_net_tcp_accept(XVal handle_value, XVal timeout_value) {
    if (handle_value.tag != XT_INT) {
        return xr_err("socket handle is not a number");
    }
    long long handle = handle_value.i;
    XrNetEntry *entry = xr_net_entry(handle);
    if (entry == NULL) {
        return xr_net_not_open(handle);
    }
    if (entry->kind != 1) {
        return xr_net_wrong_kind(handle, "stream", "listener");
    }
    long long timeout_ms = xr_net_timeout_ms(timeout_value);
    if (timeout_ms >= 0) {
        int ready = xr_net_wait(entry->fd, 0, timeout_ms);
        if (ready == 0) {
            char message[128];
            snprintf(message, sizeof(message), "accept timed out after %lld ms", timeout_ms);
            return xr_err(message);
        }
        if (ready < 0) {
            return xr_err("accept failed");
        }
    }
    struct sockaddr_storage address;
    socklen_t address_len = sizeof(address);
    XrNetFd client = accept(entry->fd, (struct sockaddr *)&address, &address_len);
    if (client == XR_NET_INVALID) {
        return xr_err("accept failed");
    }
    return xr_ok(xr_int(xr_net_register(2, client)));
}

static int xr_net_max_bytes(XVal value, long long *out) {
    if (value.tag == XT_INT) {
        *out = value.i;
        return 1;
    }
    if (value.tag == XT_FLOAT) {
        if (!isfinite(value.f)) {
            return 0;
        }
        *out = (long long)value.f;
        return 1;
    }
    return 0;
}

// Decode one read's bytes lossily: valid scalars pass through, invalid bytes
// become U+FFFD. Each call is independent, as the spec requires.
static XVal xr_net_lossy_string(const unsigned char *bytes, long long count) {
    char *out = NULL;
    long long out_len = 0;
    long long out_cap = 0;
    long long at = 0;
    while (at < count) {
        unsigned char c = bytes[at];
        int n = 0;
        int valid = 0;
        if (c < 0x80) {
            n = 1;
            valid = 1;
        } else if (c >= 0xC2 && c <= 0xDF) {
            n = 2;
        } else if (c >= 0xE0 && c <= 0xEF) {
            n = 3;
        } else if (c >= 0xF0 && c <= 0xF4) {
            n = 4;
        } else {
            n = 1;
        }
        if (!valid && n > 1 && at + n <= count) {
            valid = 1;
            for (int k = 1; k < n; k += 1) {
                if ((bytes[at + k] & 0xC0) != 0x80) {
                    valid = 0;
                    break;
                }
            }
            if (valid) {
                if (n == 3) {
                    if (c == 0xE0 && bytes[at + 1] < 0xA0) {
                        valid = 0;
                    }
                    if (c == 0xED && bytes[at + 1] >= 0xA0) {
                        valid = 0;
                    }
                } else if (n == 4) {
                    if (c == 0xF0 && bytes[at + 1] < 0x90) {
                        valid = 0;
                    }
                    if (c == 0xF4 && bytes[at + 1] >= 0x90) {
                        valid = 0;
                    }
                }
            }
        }
        if (valid) {
            xr_append_str(&out, &out_len, &out_cap, (const char *)bytes + at, n);
            at += n;
        } else {
            xr_append_str(&out, &out_len, &out_cap, "\xEF\xBF\xBD", 3);
            at += 1;
        }
    }
    return xr_str_n(out == NULL ? "" : out, out_len);
}

XVal xr_net_read(XVal handle_value, XVal max_value, XVal timeout_value) {
    long long max = 0;
    if (!xr_net_max_bytes(max_value, &max) || max < 1 || max > 16777216) {
        char message[128];
        snprintf(message, sizeof(message),
                 "read size must be between 1 and 16777216, found %lld", max);
        return xr_err(message);
    }
    if (handle_value.tag != XT_INT) {
        return xr_err("socket handle is not a number");
    }
    long long handle = handle_value.i;
    XrNetEntry *entry = xr_net_entry(handle);
    if (entry == NULL) {
        return xr_net_not_open(handle);
    }
    if (entry->kind != 2) {
        return xr_net_wrong_kind(handle, "listener", "stream");
    }
    long long timeout_ms = xr_net_timeout_ms(timeout_value);
    if (timeout_ms >= 0) {
        int ready = xr_net_wait(entry->fd, 0, timeout_ms);
        if (ready == 0) {
            char message[128];
            snprintf(message, sizeof(message), "read timed out after %lld ms", timeout_ms);
            return xr_err(message);
        }
        if (ready < 0) {
            return xr_err("read failed");
        }
    }
    char *buffer = (char *)xr_alloc((size_t)max);
#ifdef _WIN32
    int got = recv(entry->fd, buffer, (int)max, 0);
#else
    ssize_t got = recv(entry->fd, buffer, (size_t)max, 0);
#endif
    if (got == 0) {
        return xr_ok(xr_nil());
    }
    if (got < 0) {
        int code = XR_NET_LAST_ERROR;
#ifdef _WIN32
        if (code == WSAETIMEDOUT) {
#else
        if (code == EAGAIN || code == EWOULDBLOCK) {
#endif
            char message[128];
            snprintf(message, sizeof(message), "read timed out after %lld ms", timeout_ms);
            return xr_err(message);
        }
        char message[96];
        snprintf(message, sizeof(message), "read failed: error %d", code);
        return xr_err(message);
    }
    return xr_ok(xr_net_lossy_string((const unsigned char *)buffer, (long long)got));
}

XVal xr_net_write(XVal handle_value, XVal data_value, XVal timeout_value) {
    if (handle_value.tag != XT_INT) {
        return xr_err("socket handle is not a number");
    }
    if (data_value.tag != XT_STR) {
        return xr_err("std::net::sock_write expects a string");
    }
    long long handle = handle_value.i;
    XrNetEntry *entry = xr_net_entry(handle);
    if (entry == NULL) {
        return xr_net_not_open(handle);
    }
    if (entry->kind != 2) {
        return xr_net_wrong_kind(handle, "listener", "stream");
    }
    XStr *data = xr_as_str(data_value);
    long long timeout_ms = xr_net_timeout_ms(timeout_value);
    long long deadline = timeout_ms >= 0 ? xr_net_now_ms() + timeout_ms : 0;
    long long sent_total = 0;
    while (sent_total < data->bytes) {
        long long remaining = timeout_ms;
        if (timeout_ms >= 0) {
            long long now = xr_net_now_ms();
            remaining = deadline - now;
            if (remaining <= 0) {
                char message[128];
                snprintf(message, sizeof(message), "write timed out after %lld ms", timeout_ms);
                return xr_err(message);
            }
        }
        if (remaining >= 0) {
            int ready = xr_net_wait(entry->fd, 1, remaining);
            if (ready == 0) {
                char message[128];
                snprintf(message, sizeof(message), "write timed out after %lld ms", timeout_ms);
                return xr_err(message);
            }
            if (ready < 0) {
                return xr_err("write failed");
            }
        }
#ifdef _WIN32
        int sent = send(entry->fd, data->data + sent_total,
                        (int)(data->bytes - sent_total), 0);
#else
        ssize_t sent = send(entry->fd, data->data + sent_total,
                            (size_t)(data->bytes - sent_total), 0);
#endif
        if (sent <= 0) {
            int code = XR_NET_LAST_ERROR;
            char message[96];
            snprintf(message, sizeof(message), "write failed: error %d", code);
            return xr_err(message);
        }
        sent_total += (long long)sent;
    }
    return xr_ok(xr_int(data->bytes));
}

static XVal xr_net_shutdown_common(XVal handle_value, int how) {
    if (handle_value.tag != XT_INT) {
        return xr_err("socket handle is not a number");
    }
    long long handle = handle_value.i;
    XrNetEntry *entry = xr_net_entry(handle);
    if (entry == NULL) {
        return xr_net_not_open(handle);
    }
    if (entry->kind != 2) {
        return xr_net_wrong_kind(handle, "listener", "stream");
    }
    if (shutdown(entry->fd, how) != 0) {
        return xr_err("shutdown failed");
    }
    return xr_ok(xr_nil());
}

XVal xr_net_shutdown(XVal handle_value) {
    return xr_net_shutdown_common(handle_value, XR_NET_SHUT_RDWR);
}

XVal xr_net_shutdown_write(XVal handle_value) {
    return xr_net_shutdown_common(handle_value, XR_NET_SHUT_WR);
}

XVal xr_net_close(XVal handle_value) {
    if (handle_value.tag != XT_INT) {
        return xr_err("socket handle is not a number");
    }
    long long handle = handle_value.i;
    XrNetEntry *entry = xr_net_entry(handle);
    if (entry == NULL) {
        return xr_net_not_open(handle);
    }
    xr_net_close_socket(entry->fd);
    entry->kind = 0;
    entry->fd = XR_NET_INVALID;
    return xr_ok(xr_nil());
}

static int xr_net_address_text(struct sockaddr *address, socklen_t length, char *text,
                               size_t text_cap) {
    char host[256];
    char service[64];
    if (getnameinfo(address, length, host, sizeof(host), service, sizeof(service),
                    NI_NUMERICHOST | NI_NUMERICSERV) != 0) {
        return 0;
    }
    if (strchr(host, ':') != NULL) {
        snprintf(text, text_cap, "[%s]:%s", host, service);
    } else {
        snprintf(text, text_cap, "%s:%s", host, service);
    }
    return 1;
}

XVal xr_net_local_addr(XVal handle_value) {
    if (handle_value.tag != XT_INT) {
        return xr_err("socket handle is not a number");
    }
    long long handle = handle_value.i;
    XrNetEntry *entry = xr_net_entry(handle);
    if (entry == NULL) {
        return xr_net_not_open(handle);
    }
    struct sockaddr_storage address;
    socklen_t length = sizeof(address);
    if (getsockname(entry->fd, (struct sockaddr *)&address, &length) != 0) {
        return xr_err("local_addr failed");
    }
    char text[512];
    if (!xr_net_address_text((struct sockaddr *)&address, length, text, sizeof(text))) {
        return xr_err("local_addr failed");
    }
    return xr_ok(xr_str(text));
}

XVal xr_net_peer_addr(XVal handle_value) {
    if (handle_value.tag != XT_INT) {
        return xr_err("socket handle is not a number");
    }
    long long handle = handle_value.i;
    XrNetEntry *entry = xr_net_entry(handle);
    if (entry == NULL) {
        return xr_net_not_open(handle);
    }
    if (entry->kind != 2) {
        return xr_net_wrong_kind(handle, "listener", "stream");
    }
    struct sockaddr_storage address;
    socklen_t length = sizeof(address);
    if (getpeername(entry->fd, (struct sockaddr *)&address, &length) != 0) {
        return xr_err("peer_addr failed");
    }
    char text[512];
    if (!xr_net_address_text((struct sockaddr *)&address, length, text, sizeof(text))) {
        return xr_err("peer_addr failed");
    }
    return xr_ok(xr_str(text));
}
