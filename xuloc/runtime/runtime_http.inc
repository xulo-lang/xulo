// ── std::http ────────────────────────────────────────────────────────────
//
// HTTP/1.1 client and server over the `std::net` primitives
// (std-spec/modules/http.md). Requests and responses are field tables, the
// handler is a first-class function value, and the client owns one connection
// per exchange (`Connection: close`).

#define XR_HTTP_MAX_HEAD (64LL * 1024)
#define XR_HTTP_MAX_BODY (16LL * 1024 * 1024)

static XVal xr_http_pair(const char *name, XVal value) {
    XVal pair = xr_map_new();
    xr_map_set(pair, xr_str("name"), xr_str(name));
    xr_map_set(pair, xr_str("value"), value);
    return pair;
}

static XVal xr_http_list_find(XVal list, XVal name, int insensitive) {
    if (list.tag != XT_LIST || name.tag != XT_STR) {
        return xr_nil();
    }
    XStr *want = xr_as_str(name);
    XList *items = xr_as_list(list);
    for (long long i = 0; i < items->len; i += 1) {
        XVal item = items->items[i];
        if (item.tag != XT_OBJ) {
            continue;
        }
        XVal key = xr_get_field(item, "name");
        if (key.tag != XT_STR) {
            continue;
        }
        XStr *candidate = xr_as_str(key);
        int equal = 0;
        if (candidate->bytes == want->bytes) {
            if (insensitive) {
                equal = 1;
                for (long long k = 0; k < want->bytes; k += 1) {
                    char a = candidate->data[k];
                    char b = want->data[k];
                    if (a >= 'A' && a <= 'Z') {
                        a = (char)(a + 32);
                    }
                    if (b >= 'A' && b <= 'Z') {
                        b = (char)(b + 32);
                    }
                    if (a != b) {
                        equal = 0;
                        break;
                    }
                }
            } else {
                equal = memcmp(candidate->data, want->data, (size_t)want->bytes) == 0;
            }
        }
        if (equal) {
            return xr_get_field(item, "value");
        }
    }
    return xr_nil();
}

static XVal xr_http_header_value(XVal headers, const char *name) {
    return xr_http_list_find(headers, xr_str(name), 1);
}

static XVal xr_http_request_object(XVal method, XVal path, XVal query, XVal form, XVal headers,
                                   XVal body, XVal version, XVal remote) {
    const char *names[8];
    XVal values[8];
    names[0] = "method";
    names[1] = "path";
    names[2] = "query";
    names[3] = "form";
    names[4] = "headers";
    names[5] = "body";
    names[6] = "version";
    names[7] = "remote_addr";
    values[0] = method;
    values[1] = path;
    values[2] = query;
    values[3] = form;
    values[4] = headers;
    values[5] = body;
    values[6] = version;
    values[7] = remote;
    return xr_struct("Request", 8, names, values);
}

static XVal xr_http_response_object(XVal status, XVal headers, XVal body) {
    const char *names[3];
    XVal values[3];
    names[0] = "status";
    names[1] = "headers";
    names[2] = "body";
    values[0] = status;
    values[1] = headers;
    values[2] = body;
    return xr_struct("Response", 3, names, values);
}

XVal xr_http_response_text(XVal body, XVal status_value) {
    if (body.tag != XT_STR) {
        xr_fail("`Response::text` expects a string body");
    }
    long long status = 200;
    if (status_value.tag == XT_INT) {
        status = status_value.i;
    }
    XVal headers = xr_list_new();
    xr_list_push(headers, xr_http_pair("content-type", xr_str("text/plain; charset=utf-8")));
    return xr_http_response_object(xr_int(status), headers, body);
}

XVal xr_http_response_json(XVal value) {
    XVal encoded = xr_json_stringify(value, xr_nil());
    if (!xr_is_variant(encoded, "Result::Ok")) {
        return encoded;
    }
    XVal headers = xr_list_new();
    xr_list_push(headers, xr_http_pair("content-type", xr_str("application/json")));
    return xr_ok(xr_http_response_object(xr_int(200), headers, xr_payload(encoded, 0)));
}

XVal xr_http_listen(XVal addr) {
    XVal listener = xr_net_tcp_listen(addr);
    if (!xr_is_variant(listener, "Result::Ok")) {
        return listener;
    }
    const char *names[1];
    XVal values[1];
    names[0] = "handle";
    values[0] = xr_payload(listener, 0);
    return xr_ok(xr_struct("Server", 1, names, values));
}

// ── buffered reader ──────────────────────────────────────────────────────

typedef struct {
    XVal handle;
    XVal timeout;
    char *data;
    long long len;
    long long pos;
    long long cap;
    int eof;
    int failed;
    char error[256];
} XrHttpReader;

static void xr_http_reader_init(XrHttpReader *reader, XVal handle, XVal timeout) {
    memset(reader, 0, sizeof(*reader));
    reader->handle = handle;
    reader->timeout = timeout;
}

static void xr_http_reader_append(XrHttpReader *reader, const char *data, long long bytes) {
    if (reader->len + bytes + 1 > reader->cap) {
        long long next = reader->cap == 0 ? 4096 : reader->cap;
        while (next < reader->len + bytes + 1) {
            next *= 2;
        }
        char *grown = (char *)xr_alloc((size_t)next);
        if (reader->len > 0) {
            memcpy(grown, reader->data, (size_t)reader->len);
        }
        reader->data = grown;
        reader->cap = next;
    }
    memcpy(reader->data + reader->len, data, (size_t)bytes);
    reader->len += bytes;
    reader->data[reader->len] = '\0';
}

static int xr_http_reader_fill(XrHttpReader *reader) {
    if (reader->eof || reader->failed) {
        return 0;
    }
    XVal chunk = xr_net_read(reader->handle, xr_int(4096), reader->timeout);
    if (!xr_is_variant(chunk, "Result::Ok")) {
        reader->failed = 1;
        XVal error = xr_payload(chunk, 0);
        XVal message = xr_get_field(error, "message");
        snprintf(reader->error, sizeof(reader->error), "%s",
                 message.tag == XT_STR ? xr_as_str(message)->data : "read failed");
        return 0;
    }
    XVal text = xr_payload(chunk, 0);
    if (text.tag == XT_NIL) {
        reader->eof = 1;
        return 0;
    }
    XStr *str = xr_as_str(text);
    xr_http_reader_append(reader, str->data, str->bytes);
    return 1;
}

// Read up to `delim`, returning a pointer into the reader's buffer and its
// length; the delimiter is consumed. The pointer is valid until the next fill.
static int xr_http_reader_until(XrHttpReader *reader, const char *delim, long long limit,
                                const char **out, long long *out_len, const char *what) {
    long long dlen = (long long)strlen(delim);
    long long start = reader->pos;
    while (1) {
        for (long long at = start; at + dlen <= reader->len; at += 1) {
            if (memcmp(reader->data + at, delim, (size_t)dlen) == 0) {
                *out = reader->data + start;
                *out_len = at - start;
                reader->pos = at + dlen;
                return 1;
            }
        }
        if (reader->len - start > limit) {
            reader->failed = 1;
            snprintf(reader->error, sizeof(reader->error), "%s too large", what);
            return 0;
        }
        if (!xr_http_reader_fill(reader)) {
            if (!reader->failed) {
                reader->failed = 1;
                snprintf(reader->error, sizeof(reader->error), "connection closed");
            }
            return 0;
        }
    }
}

static int xr_http_reader_exact(XrHttpReader *reader, long long bytes, XVal *out) {
    while (reader->len - reader->pos < bytes) {
        if (!xr_http_reader_fill(reader)) {
            if (!reader->failed) {
                reader->failed = 1;
                snprintf(reader->error, sizeof(reader->error), "connection closed");
            }
            return 0;
        }
    }
    *out = xr_str_n(reader->data + reader->pos, bytes);
    reader->pos += bytes;
    return 1;
}

static int xr_http_reader_all(XrHttpReader *reader, XVal *out, long long limit) {
    while (xr_http_reader_fill(reader)) {
        if (reader->len - reader->pos > limit) {
            reader->failed = 1;
            snprintf(reader->error, sizeof(reader->error), "body too large");
            return 0;
        }
    }
    if (reader->failed) {
        return 0;
    }
    *out = xr_str_n(reader->data + reader->pos, reader->len - reader->pos);
    reader->pos = reader->len;
    return 1;
}

static int xr_http_reader_chunked(XrHttpReader *reader, XVal *out) {
    char *body = NULL;
    long long body_len = 0;
    long long body_cap = 0;
    while (1) {
        const char *line = NULL;
        long long line_len = 0;
        if (!xr_http_reader_until(reader, "\r\n", 1024, &line, &line_len, "chunk size")) {
            return 0;
        }
        long long size = 0;
        int digits = 0;
        for (long long i = 0; i < line_len; i += 1) {
            char c = line[i];
            if (c == ';') {
                break;
            }
            int value = -1;
            if (c >= '0' && c <= '9') {
                value = c - '0';
            } else if (c >= 'a' && c <= 'f') {
                value = c - 'a' + 10;
            } else if (c >= 'A' && c <= 'F') {
                value = c - 'A' + 10;
            } else {
                continue;
            }
            size = size * 16 + value;
            digits += 1;
            if (size > XR_HTTP_MAX_BODY) {
                reader->failed = 1;
                snprintf(reader->error, sizeof(reader->error), "body too large");
                return 0;
            }
        }
        if (digits == 0) {
            reader->failed = 1;
            snprintf(reader->error, sizeof(reader->error), "invalid chunk size");
            return 0;
        }
        if (size == 0) {
            // Discard trailers up to the blank line.
            while (1) {
                if (!xr_http_reader_until(reader, "\r\n", 1024, &line, &line_len, "chunk trailer")) {
                    return 0;
                }
                if (line_len == 0) {
                    break;
                }
            }
            break;
        }
        XVal chunk = xr_nil();
        if (!xr_http_reader_exact(reader, size, &chunk)) {
            return 0;
        }
        XStr *text = xr_as_str(chunk);
        xr_append_str(&body, &body_len, &body_cap, text->data, text->bytes);
        if (!xr_http_reader_exact(reader, 2, &chunk)) {
            return 0;
        }
    }
    *out = xr_str_n(body == NULL ? "" : body, body_len);
    return 1;
}

// ── response helpers ─────────────────────────────────────────────────────

static int xr_http_parse_int(const char *text, long long len, long long *out) {
    long long value = 0;
    if (len == 0) {
        return 0;
    }
    for (long long i = 0; i < len; i += 1) {
        if (text[i] < '0' || text[i] > '9') {
            return 0;
        }
        value = value * 10 + (text[i] - '0');
        if (value > XR_HTTP_MAX_BODY * 100) {
            return 0;
        }
    }
    *out = value;
    return 1;
}

static void xr_http_trim(const char **text, long long *len) {
    while (*len > 0 && (**text == ' ' || **text == '\t')) {
        *text += 1;
        *len -= 1;
    }
    while (*len > 0 && ((*text)[*len - 1] == ' ' || (*text)[*len - 1] == '\t')) {
        *len -= 1;
    }
}

static int xr_http_header_name_ok(const char *name, long long len) {
    if (len == 0) {
        return 0;
    }
    for (long long i = 0; i < len; i += 1) {
        unsigned char c = (unsigned char)name[i];
        if (c <= 0x20 || c >= 0x7F || c == ':') {
            return 0;
        }
    }
    return 1;
}

static int xr_http_header_value_ok(const char *value, long long len) {
    for (long long i = 0; i < len; i += 1) {
        unsigned char c = (unsigned char)value[i];
        if (c == '\r' || c == '\n' || (c < 0x20 && c != '\t')) {
            return 0;
        }
    }
    return 1;
}

static XVal xr_http_parse_header_lines(const char *data, long long len, int *ok) {
    XVal headers = xr_list_new();
    *ok = 1;
    long long at = 0;
    while (at < len) {
        long long end = at;
        while (end + 1 < len && !(data[end] == '\r' && data[end + 1] == '\n')) {
            end += 1;
        }
        int has_crlf = end + 1 < len;
        long long line_len = has_crlf ? end - at : len - at;
        const char *line = data + at;
        if (line_len == 0) {
            break;
        }
        long long colon = -1;
        for (long long i = 0; i < line_len; i += 1) {
            if (line[i] == ':') {
                colon = i;
                break;
            }
        }
        if (colon <= 0) {
            *ok = 0;
            return headers;
        }
        const char *name = line;
        long long name_len = colon;
        const char *value = line + colon + 1;
        long long value_len = line_len - colon - 1;
        while (value_len > 0 && (*value == ' ' || *value == '\t')) {
            value += 1;
            value_len -= 1;
        }
        while (value_len > 0 && (value[value_len - 1] == ' ' || value[value_len - 1] == '\t')) {
            value_len -= 1;
        }
        if (!xr_http_header_name_ok(name, name_len) ||
            !xr_http_header_value_ok(value, value_len)) {
            *ok = 0;
            return headers;
        }
        XVal pair = xr_map_new();
        xr_map_set(pair, xr_str("name"), xr_str_n(name, name_len));
        xr_map_set(pair, xr_str("value"), xr_str_n(value, value_len));
        xr_list_push(headers, pair);
        if (!has_crlf) {
            break;
        }
        at = end + 2;
    }
    return headers;
}

static const char *xr_http_reason(long long status) {
    switch (status) {
    case 200:
        return "OK";
    case 201:
        return "Created";
    case 204:
        return "No Content";
    case 304:
        return "Not Modified";
    case 400:
        return "Bad Request";
    case 404:
        return "Not Found";
    case 413:
        return "Payload Too Large";
    case 417:
        return "Expectation Failed";
    case 431:
        return "Request Header Fields Too Large";
    case 500:
        return "Internal Server Error";
    case 501:
        return "Not Implemented";
    case 505:
        return "HTTP Version Not Supported";
    default:
        return "OK";
    }
}

// ── client ───────────────────────────────────────────────────────────────

static int xr_http_url_ok(const char *url) {
    for (const char *p = url; *p != '\0'; p += 1) {
        unsigned char c = (unsigned char)*p;
        if (c <= 0x20 || c >= 0x7F) {
            return 0;
        }
    }
    return 1;
}

static int xr_http_parse_url(const char *url, char *authority, size_t authority_cap,
                             char *host_port, size_t host_port_cap, char *target,
                             size_t target_cap, char *error, size_t error_cap) {
    if (!xr_http_url_ok(url)) {
        snprintf(error, error_cap, "URL contains a character that must be percent-encoded");
        return 0;
    }
    if (strlen(url) < 7 || strncasecmp(url, "http://", 7) != 0) {
        snprintf(error, error_cap, "only the `http` scheme is supported");
        return 0;
    }
    const char *rest = url + 7;
    const char *end = rest;
    while (*end != '\0' && *end != '/' && *end != '?' && *end != '#') {
        end += 1;
    }
    size_t authority_len = (size_t)(end - rest);
    if (authority_len == 0 || authority_len >= authority_cap) {
        snprintf(error, error_cap, "empty host in URL");
        return 0;
    }
    memcpy(authority, rest, authority_len);
    authority[authority_len] = '\0';
    // host[:port] (bracketed for IPv6); port defaults to 80.
    const char *port = "80";
    char port_buffer[16];
    int has_port = 0;
    if (authority[0] == '[') {
        char *close = strchr(authority, ']');
        if (close == NULL) {
            snprintf(error, error_cap, "malformed IPv6 host");
            return 0;
        }
        if (close[1] == ':') {
            has_port = 1;
            snprintf(port_buffer, sizeof(port_buffer), "%s", close + 2);
            port = port_buffer;
        } else if (close[1] != '\0') {
            snprintf(error, error_cap, "malformed host");
            return 0;
        }
    } else {
        char *colon = strrchr(authority, ':');
        if (colon != NULL) {
            has_port = 1;
            snprintf(port_buffer, sizeof(port_buffer), "%s", colon + 1);
            port = port_buffer;
        }
    }
    long long port_number = 0;
    long long port_len = (long long)strlen(port);
    if (!xr_http_parse_int(port, port_len, &port_number) || port_number < 1 ||
        port_number > 65535) {
        snprintf(error, error_cap, "invalid port in URL");
        return 0;
    }
    if (has_port) {
        snprintf(host_port, host_port_cap, "%s", authority);
    } else {
        snprintf(host_port, host_port_cap, "%s:80", authority);
    }
    // target: path (default `/`) plus `?query`, fragment dropped.
    const char *path = end;
    if (*path == '\0' || *path == '#' || *path == '?') {
        path = "/";
    }
    const char *fragment = strchr(end, '#');
    const char *target_end = fragment == NULL ? end + strlen(end) : fragment;
    size_t target_len = (size_t)(target_end - end);
    if (target_len == 0) {
        snprintf(target, target_cap, "/");
    } else if (*end != '/') {
        snprintf(target, target_cap, "/%s", end);
    } else {
        if (target_len >= target_cap) {
            snprintf(error, error_cap, "URL path is too long");
            return 0;
        }
        memcpy(target, end, target_len);
        target[target_len] = '\0';
    }
    return 1;
}

static int xr_http_header_skip(const char *name, long long len) {
    if ((len == 10 && strncasecmp(name, "connection", 10) == 0) ||
        (len == 14 && strncasecmp(name, "content-length", 14) == 0) ||
        (len == 17 && strncasecmp(name, "transfer-encoding", 17) == 0)) {
        return 1;
    }
    return 0;
}

static XVal xr_http_client_request(XVal method_value, XVal url_value, XVal headers_value,
                                   XVal body_value, XVal timeout_value) {
    if (method_value.tag != XT_STR || url_value.tag != XT_STR) {
        return xr_err("`std::http::request` expects a method and a URL string");
    }
    XStr *method = xr_as_str(method_value);
    XStr *url = xr_as_str(url_value);
    if (method->bytes == 0) {
        return xr_err("HTTP method must not be empty");
    }
    for (long long i = 0; i < method->bytes; i += 1) {
        unsigned char c = (unsigned char)method->data[i];
        if (c == ' ' || c < 0x20 || c >= 0x7F) {
            return xr_err("HTTP method contains an invalid character");
        }
    }
    char authority[512];
    char host_port[512];
    char target[4096];
    char error[512];
    if (!xr_http_parse_url(url->data, authority, sizeof(authority), host_port, sizeof(host_port),
                           target, sizeof(target), error, sizeof(error))) {
        return xr_err(error);
    }
    XVal connected = xr_net_tcp_connect(xr_str(host_port), timeout_value);
    if (!xr_is_variant(connected, "Result::Ok")) {
        return connected;
    }
    XVal handle = xr_payload(connected, 0);
    char *head = NULL;
    long long head_len = 0;
    long long head_cap = 0;
    xr_append_str(&head, &head_len, &head_cap, method->data, method->bytes);
    xr_append_str(&head, &head_len, &head_cap, " ", 1);
    xr_append_str(&head, &head_len, &head_cap, target, (long long)strlen(target));
    xr_append_str(&head, &head_len, &head_cap, " HTTP/1.1\r\n", 11);
    int caller_host = 0;
    if (headers_value.tag == XT_LIST) {
        XList *headers = xr_as_list(headers_value);
        for (long long i = 0; i < headers->len; i += 1) {
            XVal item = headers->items[i];
            if (item.tag == XT_OBJ) {
                XVal name = xr_get_field(item, "name");
                if (name.tag == XT_STR && xr_as_str(name)->bytes == 4 &&
                    strncasecmp(xr_as_str(name)->data, "host", 4) == 0) {
                    caller_host = 1;
                }
            }
        }
    }
    if (!caller_host) {
        xr_append_str(&head, &head_len, &head_cap, "Host: ", 6);
        xr_append_str(&head, &head_len, &head_cap, authority, (long long)strlen(authority));
        xr_append_str(&head, &head_len, &head_cap, "\r\n", 2);
    }
    xr_append_str(&head, &head_len, &head_cap, "Connection: close\r\n", 19);
    if (headers_value.tag == XT_LIST) {
        XList *headers = xr_as_list(headers_value);
        for (long long i = 0; i < headers->len; i += 1) {
            XVal item = headers->items[i];
            if (item.tag != XT_OBJ) {
                continue;
            }
            XVal name = xr_get_field(item, "name");
            XVal value = xr_get_field(item, "value");
            if (name.tag != XT_STR || value.tag != XT_STR) {
                continue;
            }
            XStr *n = xr_as_str(name);
            XStr *v = xr_as_str(value);
            if (xr_http_header_skip(n->data, n->bytes)) {
                continue;
            }
            if (!xr_http_header_name_ok(n->data, n->bytes) ||
                !xr_http_header_value_ok(v->data, v->bytes)) {
                xr_net_close(handle);
                return xr_err("HTTP header contains an invalid character");
            }
            xr_append_str(&head, &head_len, &head_cap, n->data, n->bytes);
            xr_append_str(&head, &head_len, &head_cap, ": ", 2);
            xr_append_str(&head, &head_len, &head_cap, v->data, v->bytes);
            xr_append_str(&head, &head_len, &head_cap, "\r\n", 2);
        }
    }
    long long body_bytes = body_value.tag == XT_STR ? xr_as_str(body_value)->bytes : 0;
    if (body_value.tag == XT_STR) {
        char length[64];
        int n = snprintf(length, sizeof(length), "Content-Length: %lld\r\n", body_bytes);
        xr_append_str(&head, &head_len, &head_cap, length, n);
    }
    xr_append_str(&head, &head_len, &head_cap, "\r\n", 2);
    if (body_value.tag == XT_STR) {
        XStr *body = xr_as_str(body_value);
        xr_append_str(&head, &head_len, &head_cap, body->data, body->bytes);
    }
    XVal written = xr_net_write(handle, xr_str_n(head, head_len), timeout_value);
    if (!xr_is_variant(written, "Result::Ok")) {
        xr_net_close(handle);
        return written;
    }
    XrHttpReader reader;
    xr_http_reader_init(&reader, handle, timeout_value);
    const char *head_text = NULL;
    long long header_len = 0;
    if (!xr_http_reader_until(&reader, "\r\n\r\n", XR_HTTP_MAX_HEAD, &head_text, &header_len,
                              "response headers")) {
        XVal error_value = xr_err(reader.error);
        xr_net_close(handle);
        return error_value;
    }
    // Status line.
    long long line_end = 0;
    while (line_end + 1 < header_len &&
           !(head_text[line_end] == '\r' && head_text[line_end + 1] == '\n')) {
        line_end += 1;
    }
    if (line_end < 8 || strncasecmp(head_text, "HTTP/1.", 7) != 0) {
        xr_net_close(handle);
        return xr_err("malformed HTTP status line");
    }
    long long status = 0;
    long long at = 7;
    at += 1; // minor version digit
    while (at < line_end && head_text[at] == ' ') {
        at += 1;
    }
    long long status_end = at;
    while (status_end < line_end && head_text[status_end] >= '0' && head_text[status_end] <= '9') {
        status_end += 1;
    }
    if (!xr_http_parse_int(head_text + at, status_end - at, &status) || status < 200 ||
        status > 599) {
        xr_net_close(handle);
        return xr_err("malformed HTTP status line");
    }
    int ok = 1;
    long long headers_at = line_end + 2;
    XVal headers = xr_http_parse_header_lines(head_text + headers_at, header_len - headers_at, &ok);
    if (!ok) {
        xr_net_close(handle);
        return xr_err("malformed HTTP response header");
    }
    XVal body = xr_str("");
    int no_body = status == 204 || status == 304 ||
                  (method->bytes == 4 && strncasecmp(method->data, "HEAD", 4) == 0);
    XVal transfer = xr_http_header_value(headers, "transfer-encoding");
    XVal length_value = xr_http_header_value(headers, "content-length");
    if (!no_body) {
        if (transfer.tag == XT_STR && strncasecmp(xr_as_str(transfer)->data, "chunked", 7) == 0) {
            if (!xr_http_reader_chunked(&reader, &body)) {
                XVal error_value = xr_err(reader.error);
                xr_net_close(handle);
                return error_value;
            }
        } else if (length_value.tag == XT_STR) {
            long long length = 0;
            if (!xr_http_parse_int(xr_as_str(length_value)->data,
                                   (long long)strlen(xr_as_str(length_value)->data), &length) ||
                length > XR_HTTP_MAX_BODY) {
                xr_net_close(handle);
                return xr_err("invalid Content-Length");
            }
            if (!xr_http_reader_exact(&reader, length, &body)) {
                XVal error_value = xr_err(reader.error);
                xr_net_close(handle);
                return error_value;
            }
        } else {
            if (!xr_http_reader_all(&reader, &body, XR_HTTP_MAX_BODY)) {
                XVal error_value = xr_err(reader.error);
                xr_net_close(handle);
                return error_value;
            }
        }
    }
    xr_net_close(handle);
    XVal lossy = xr_net_lossy_string((const unsigned char *)xr_as_str(body)->data,
                                     xr_as_str(body)->bytes);
    return xr_ok(xr_http_response_object(xr_int(status), headers, lossy));
}

XVal xr_http_fetch(XVal url, XVal timeout) {
    return xr_http_client_request(xr_str("GET"), url, xr_nil(), xr_nil(), timeout);
}

XVal xr_http_request(XVal method, XVal url, XVal headers, XVal body, XVal timeout) {
    return xr_http_client_request(method, url, headers, body, timeout);
}

// ── server ───────────────────────────────────────────────────────────────

static int xr_http_percent_decode(const char *text, long long len, int plus_space, char **out,
                                  long long *out_len, const char *error, size_t error_cap) {
    char *decoded = (char *)xr_alloc((size_t)len + 1);
    long long at = 0;
    long long written = 0;
    while (at < len) {
        unsigned char c = (unsigned char)text[at];
        if (c == '%') {
            if (at + 2 >= len) {
                snprintf(error, error_cap, "malformed percent escape");
                return 0;
            }
            int hi = -1;
            int lo = -1;
            unsigned char a = (unsigned char)text[at + 1];
            unsigned char b = (unsigned char)text[at + 2];
            if (a >= '0' && a <= '9') hi = a - '0';
            else if (a >= 'a' && a <= 'f') hi = a - 'a' + 10;
            else if (a >= 'A' && a <= 'F') hi = a - 'A' + 10;
            if (b >= '0' && b <= '9') lo = b - '0';
            else if (b >= 'a' && b <= 'f') lo = b - 'a' + 10;
            else if (b >= 'A' && b <= 'F') lo = b - 'A' + 10;
            if (hi < 0 || lo < 0) {
                snprintf(error, error_cap, "malformed percent escape");
                return 0;
            }
            decoded[written++] = (char)(hi * 16 + lo);
            at += 3;
        } else if (c == '+' && plus_space) {
            decoded[written++] = ' ';
            at += 1;
        } else {
            decoded[written++] = (char)c;
            at += 1;
        }
    }
    decoded[written] = '\0';
    *out = decoded;
    *out_len = written;
    return 1;
}

static XVal xr_http_parse_pairs(const char *text, long long len, int plus_space, int *ok,
                                const char *error, size_t error_cap) {
    XVal list = xr_list_new();
    *ok = 1;
    long long at = 0;
    while (at <= len) {
        long long end = at;
        while (end < len && text[end] != '&') {
            end += 1;
        }
        if (end > at) {
            long long eq = -1;
            for (long long i = at; i < end; i += 1) {
                if (text[i] == '=') {
                    eq = i;
                    break;
                }
            }
            long long name_start = at;
            long long name_end = eq < 0 ? end : eq;
            long long value_start = eq < 0 ? end : eq + 1;
            char *name = NULL;
            char *value = NULL;
            long long name_len = 0;
            long long value_len = 0;
            if (!xr_http_percent_decode(text + name_start, name_end - name_start, plus_space,
                                        &name, &name_len, error, error_cap) ||
                !xr_http_percent_decode(text + value_start, end - value_start, plus_space,
                                        &value, &value_len, error, error_cap)) {
                *ok = 0;
                return list;
            }
            XVal pair = xr_map_new();
            xr_map_set(pair, xr_str("name"), xr_str_n(name, name_len));
            xr_map_set(pair, xr_str("value"), xr_str_n(value, value_len));
            xr_list_push(list, pair);
        }
        if (end >= len) {
            break;
        }
        at = end + 1;
    }
    return list;
}

static int xr_http_send_error(XVal client, long long status) {
    char response[256];
    int n = snprintf(response, sizeof(response),
                     "HTTP/1.1 %lld %s\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", status,
                     xr_http_reason(status));
    XVal written = xr_net_write(client, xr_str_n(response, n), xr_nil());
    xr_net_close(client);
    return xr_is_variant(written, "Result::Ok");
}

static int xr_http_write_response(XVal client, XVal response, const char *method, long long method_len) {
    if (response.tag != XT_OBJ || strcmp(xr_as_obj(response)->type, "Response") != 0) {
        return xr_http_send_error(client, 500);
    }
    XVal status_value = xr_get_field(response, "status");
    long long status = status_value.tag == XT_INT ? status_value.i : 200;
    if (status < 200 || status > 599) {
        status = 500;
    }
    XVal headers = xr_get_field(response, "headers");
    XVal body_value = xr_get_field(response, "body");
    XStr *body = body_value.tag == XT_STR ? xr_as_str(body_value) : NULL;
    char *head = NULL;
    long long head_len = 0;
    long long head_cap = 0;
    char line[128];
    int n = snprintf(line, sizeof(line), "HTTP/1.1 %lld %s\r\n", status, xr_http_reason(status));
    xr_append_str(&head, &head_len, &head_cap, line, n);
    if (headers.tag == XT_LIST) {
        XList *list = xr_as_list(headers);
        for (long long i = 0; i < list->len; i += 1) {
            XVal item = list->items[i];
            if (item.tag != XT_OBJ) {
                continue;
            }
            XVal name = xr_get_field(item, "name");
            XVal value = xr_get_field(item, "value");
            if (name.tag != XT_STR || value.tag != XT_STR) {
                continue;
            }
            if (xr_http_header_skip(xr_as_str(name)->data, xr_as_str(name)->bytes)) {
                continue;
            }
            xr_append_str(&head, &head_len, &head_cap, xr_as_str(name)->data,
                          xr_as_str(name)->bytes);
            xr_append_str(&head, &head_len, &head_cap, ": ", 2);
            xr_append_str(&head, &head_len, &head_cap, xr_as_str(value)->data,
                          xr_as_str(value)->bytes);
            xr_append_str(&head, &head_len, &head_cap, "\r\n", 2);
        }
    }
    int no_body = status == 204 || status == 304;
    if (!no_body) {
        n = snprintf(line, sizeof(line), "Content-Length: %lld\r\n", body == NULL ? 0 : body->bytes);
        xr_append_str(&head, &head_len, &head_cap, line, n);
    }
    xr_append_str(&head, &head_len, &head_cap, "Connection: close\r\n\r\n", 21);
    int is_head = method_len == 4 && strncasecmp(method, "HEAD", 4) == 0;
    if (!no_body && !is_head && body != NULL) {
        xr_append_str(&head, &head_len, &head_cap, body->data, body->bytes);
    }
    XVal written = xr_net_write(client, xr_str_n(head, head_len), xr_nil());
    xr_net_close(client);
    return xr_is_variant(written, "Result::Ok");
}

XVal xr_http_serve(XVal server, XVal handler) {
    XVal handle = xr_get_field(server, "handle");
    while (1) {
        XVal accepted = xr_net_tcp_accept(handle, xr_nil());
        if (!xr_is_variant(accepted, "Result::Ok")) {
            return accepted;
        }
        XVal client = xr_payload(accepted, 0);
        XrHttpReader reader;
        xr_http_reader_init(&reader, client, xr_nil());
        const char *head_text = NULL;
        long long head_len = 0;
        if (!xr_http_reader_until(&reader, "\r\n\r\n", XR_HTTP_MAX_HEAD, &head_text, &head_len,
                                  "request headers")) {
            long long status = 400;
            if (strstr(reader.error, "too large") != NULL) {
                status = 431;
            }
            xr_http_send_error(client, status);
            continue;
        }
        long long line_end = 0;
        while (line_end + 1 < head_len &&
               !(head_text[line_end] == '\r' && head_text[line_end + 1] == '\n')) {
            line_end += 1;
        }
        long long first_space = -1;
        long long second_space = -1;
        for (long long i = 0; i < line_end; i += 1) {
            if (head_text[i] == ' ') {
                if (first_space < 0) {
                    first_space = i;
                } else {
                    second_space = i;
                    break;
                }
            }
        }
        if (first_space <= 0 || second_space <= first_space) {
            xr_http_send_error(client, 400);
            continue;
        }
        long long method_len = first_space;
        long long target_len = second_space - first_space - 1;
        long long version_len = line_end - second_space - 1;
        const char *method = head_text;
        const char *target_text = head_text + first_space + 1;
        const char *version = head_text + second_space + 1;
        if (version_len != 8 || memcmp(version, "HTTP/1.1", 8) != 0) {
            xr_http_send_error(client, 505);
            continue;
        }
        int ok = 1;
        XVal headers = xr_http_parse_header_lines(head_text + line_end + 2,
                                                 head_len - line_end - 2, &ok);
        if (!ok) {
            xr_http_send_error(client, 400);
            continue;
        }
        XVal host = xr_http_header_value(headers, "host");
        if (host.tag != XT_STR || xr_as_str(host)->bytes == 0 ||
            memchr(xr_as_str(host)->data, ' ', (size_t)xr_as_str(host)->bytes) != NULL) {
            xr_http_send_error(client, 400);
            continue;
        }
        XVal transfer = xr_http_header_value(headers, "transfer-encoding");
        if (transfer.tag != XT_NIL) {
            xr_http_send_error(client, 501);
            continue;
        }
        long long body_len = 0;
        XVal length_value = xr_http_header_value(headers, "content-length");
        if (length_value.tag == XT_STR) {
            XStr *text = xr_as_str(length_value);
            if (!xr_http_parse_int(text->data, text->bytes, &body_len)) {
                xr_http_send_error(client, 400);
                continue;
            }
            if (body_len > XR_HTTP_MAX_BODY) {
                xr_http_send_error(client, 413);
                continue;
            }
        }
        XVal expect = xr_http_header_value(headers, "expect");
        if (expect.tag == XT_STR) {
            if (body_len > 0 && xr_as_str(expect)->bytes == 12 &&
                strncasecmp(xr_as_str(expect)->data, "100-continue", 12) == 0) {
                XVal interim = xr_net_write(client, xr_str("HTTP/1.1 100 Continue\r\n\r\n"), xr_nil());
                if (!xr_is_variant(interim, "Result::Ok")) {
                    xr_net_close(client);
                    continue;
                }
            } else {
                xr_http_send_error(client, 417);
                continue;
            }
        }
        XVal body = xr_str("");
        if (body_len > 0) {
            if (!xr_http_reader_exact(&reader, body_len, &body)) {
                xr_http_send_error(client, 400);
                continue;
            }
        }
        // Path and query.
        const char *question = NULL;
        for (long long i = 0; i < target_len; i += 1) {
            if (target_text[i] == '?') {
                question = target_text + i;
                break;
            }
        }
        long long path_len = question == NULL ? target_len : (long long)(question - target_text);
        if (path_len == 0 || target_text[0] != '/') {
            xr_http_send_error(client, 400);
            continue;
        }
        char *path = NULL;
        long long path_out = 0;
        if (!xr_http_percent_decode(target_text, path_len, 0, &path, &path_out, reader.error,
                                    sizeof(reader.error))) {
            xr_http_send_error(client, 400);
            continue;
        }
        XVal query = xr_list_new();
        if (question != NULL) {
            const char *query_text = question + 1;
            long long query_len = target_len - path_len - 1;
            query = xr_http_parse_pairs(query_text, query_len, 0, &ok, reader.error,
                                        sizeof(reader.error));
            if (!ok) {
                xr_http_send_error(client, 400);
                continue;
            }
        }
        // Form decoding.
        XVal form = xr_list_new();
        XVal content_type = xr_http_header_value(headers, "content-type");
        if (content_type.tag == XT_STR && xr_as_str(content_type)->bytes >= 33 &&
            strncasecmp(xr_as_str(content_type)->data, "application/x-www-form-urlencoded", 33) == 0) {
            if (body.tag == XT_STR) {
                XStr *body_text = xr_as_str(body);
                form = xr_http_parse_pairs(body_text->data, body_text->bytes, 1, &ok, reader.error,
                                           sizeof(reader.error));
                if (!ok) {
                    xr_http_send_error(client, 400);
                    continue;
                }
            }
        }
        XVal remote = xr_net_peer_addr(client);
        XVal remote_addr = xr_is_variant(remote, "Result::Ok") ? xr_payload(remote, 0) : xr_str("");
        XVal request = xr_http_request_object(xr_str_n(method, method_len),
                                              xr_str_n(path, path_out), query, form, headers,
                                              body, xr_str("HTTP/1.1"), remote_addr);
        XVal response = xr_call_value(handler, 1, &request);
        xr_http_write_response(client, response, method, method_len);
    }
}

XVal xr_http_request_method(XVal recv, const char *name, long long n, XVal *args) {
    if (strcmp(name, "header") == 0) {
        if (n != 1 || args[0].tag != XT_STR) {
            return xr_err("`Request::header` expects a name");
        }
        return xr_http_header_value(xr_get_field(recv, "headers"), xr_as_str(args[0])->data);
    }
    if (strcmp(name, "query_value") == 0) {
        if (n != 1) {
            return xr_err("`Request::query_value` expects a name");
        }
        return xr_http_list_find(xr_get_field(recv, "query"), args[0], 0);
    }
    if (strcmp(name, "form_value") == 0) {
        if (n != 1) {
            return xr_err("`Request::form_value` expects a name");
        }
        return xr_http_list_find(xr_get_field(recv, "form"), args[0], 0);
    }
    return xr_err("unknown `Request` method");
}

XVal xr_http_response_method(XVal recv, const char *name, long long n, XVal *args) {
    if (strcmp(name, "header") == 0) {
        if (n != 1 || args[0].tag != XT_STR) {
            return xr_err("`Response::header` expects a name");
        }
        return xr_http_header_value(xr_get_field(recv, "headers"), xr_as_str(args[0])->data);
    }
    return xr_err("unknown `Response` method");
}

XVal xr_http_server_method(XVal recv, const char *name, long long n, XVal *args) {
    if (strcmp(name, "addr") == 0) {
        return xr_net_local_addr(xr_get_field(recv, "handle"));
    }
    if (strcmp(name, "serve") == 0) {
        if (n != 1) {
            return xr_err("`Server::serve` expects a handler");
        }
        return xr_http_serve(recv, args[0]);
    }
    return xr_err("unknown `Server` method");
}
