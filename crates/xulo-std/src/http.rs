//! The `http` namespace (spec `std-spec/modules/http.md`): the HTTP/1.1
//! client (`fetch`/`request`) and server (`listen` + `Server::serve`), a
//! protocol layer over plain TCP. Version 0.1 is `http://` only — `https://`
//! and every other scheme are `Err`, never a silent fallback — and bodies
//! are text in both directions (received bytes decode lossily as UTF-8).
//!
//! Failures are ordinary values: a malformed URL, a refused connection, an
//! expired budget, or a protocol violation is `Err`; a header that is not
//! there is `None` (the language sees `null`). Caps: the header block is
//! 64 KiB and a body 16 MiB on both sides.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// One `{ name, value }` record — a header, a query pair, or a form part,
/// kept in wire order with duplicates preserved.
#[derive(Clone, Debug)]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// One request as the handler sees it (spec, "Types"): the eight public
/// fields, with `query`/`form`/`headers` as `{ name, value }` records.
#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Vec<Header>,
    pub form: Vec<Header>,
    pub headers: Vec<Header>,
    pub body: String,
    pub version: String,
    pub remote_addr: String,
}

impl Request {
    /// `Request::header(name)`: the first header of that name
    /// (case-insensitive), `None` when absent.
    pub fn header(&self, name: &str) -> Option<String> {
        find_header(&self.headers, name).map(|h| h.value.clone())
    }

    /// `Request::queryValue(name)`: the first query pair of that exact name,
    /// `None` when absent.
    pub fn query_value(&self, name: &str) -> Option<String> {
        find_pair(&self.query, name).cloned()
    }

    /// `Request::formValue(name)`: the first form part of that exact name —
    /// decoded only for a form body — `None` when absent.
    pub fn form_value(&self, name: &str) -> Option<String> {
        find_pair(&self.form, name).cloned()
    }
}

/// One response (spec, "Types"): the three public fields. `Response.text`
/// and `Response.json` are shorthands that pre-set `Content-Type`; a
/// hand-built `Response` writes exactly the headers given.
#[derive(Clone, Debug)]
pub struct Response {
    pub status: i64,
    pub headers: Vec<Header>,
    pub body: String,
}

impl Response {
    /// `Response.text(body[, status])`: `text/plain; charset=utf-8`, status
    /// defaulting to `200`.
    pub fn text(body: &str, status: i64) -> Response {
        Response {
            status,
            headers: vec![Header {
                name: "Content-Type".to_string(),
                value: "text/plain; charset=utf-8".to_string(),
            }],
            body: body.to_string(),
        }
    }

    /// `Response.json(text[, status])`: the already-stringified JSON text as
    /// an `application/json` body (stringification is `json.md`'s).
    pub fn json(text: &str, status: i64) -> Response {
        Response {
            status,
            headers: vec![Header {
                name: "Content-Type".to_string(),
                value: "application/json".to_string(),
            }],
            body: text.to_string(),
        }
    }

    /// `Response::header(name)`: the first header of that name
    /// (case-insensitive), `None` when absent.
    pub fn header(&self, name: &str) -> Option<String> {
        find_header(&self.headers, name).map(|h| h.value.clone())
    }
}

/// The first header of `name` (ASCII case-insensitive).
pub fn find_header<'a>(headers: &'a [Header], name: &str) -> Option<&'a Header> {
    headers.iter().find(|h| h.name.eq_ignore_ascii_case(name))
}

/// The first `{ name, value }` pair of that exact name (query and form
/// lookup are exact — spec, "Types").
fn find_pair<'a>(pairs: &'a [Header], name: &str) -> Option<&'a String> {
    pairs.iter().find(|p| p.name == name).map(|p| &p.value)
}

// ──────────────────────────────────────────────────────────────────
//  Percent-decoding and URLs
// ──────────────────────────────────────────────────────────────────

/// Decode `%XX` escapes. `plus_as_space` is the form-body rule (a `+` is a
/// space); URL syntax keeps `+` as itself. A malformed escape is `Err`.
fn percent_decode(raw: &str, plus_as_space: bool) -> Result<String, String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' if plus_as_space => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                let hi = bytes.get(i + 1).copied();
                let lo = bytes.get(i + 2).copied();
                match (
                    hi.and_then(|b| (b as char).to_digit(16)),
                    lo.and_then(|b| (b as char).to_digit(16)),
                ) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                    }
                    _ => return Err(format!("malformed escape in `{raw}`")),
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| format!("malformed escape in `{raw}`"))
}

/// One parsed client URL: where to connect, and the request target to send.
pub struct Url {
    /// The host as it appears in the `Host` header (brackets kept for IPv6).
    pub host: String,
    /// The host without brackets — what the resolver wants.
    pub connect_host: String,
    pub port: u16,
    /// The request target: path plus `?query` as written. The fragment is
    /// dropped; a missing path is `/`.
    pub target: String,
}

/// Parse `http://host[:port][/path][?query][#fragment]` per spec, "URLs":
/// only `http` (case-insensitive scheme), port `0` refused, an empty host
/// refused, and a space/control/non-ASCII byte anywhere refused — the caller
/// percent-encodes what the URL requires.
pub fn parse_url(url: &str) -> Result<Url, String> {
    if url.bytes().any(|b| b <= 0x20 || b >= 0x7f) {
        return Err(format!(
            "URL contains a space, a control character, or a non-ASCII byte: `{url}`"
        ));
    }
    let rest = if url.len() >= 7 && url[..7].eq_ignore_ascii_case("http://") {
        &url[7..]
    } else if url.len() >= 8 && url[..8].eq_ignore_ascii_case("https://") {
        return Err("unsupported scheme `https` (version 0.1 is plain `http://` only)".to_string());
    } else if url.contains("://") {
        let scheme = url.split("://").next().unwrap_or("");
        return Err(format!("unsupported scheme `{scheme}` (only `http` is accepted)"));
    } else {
        return Err(format!("URL must start with `http://`: `{url}`"));
    };
    // The authority runs until the path, query, or fragment begins.
    let cut = rest
        .find(['/', '?', '#'])
        .unwrap_or(rest.len());
    let authority = &rest[..cut];
    let remainder = &rest[cut..];
    if authority.is_empty() {
        return Err(format!("URL has an empty host: `{url}`"));
    }
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        // A bracketed IPv6 literal, optionally followed by `:port`.
        let Some(close) = inner.find(']') else {
            return Err(format!("malformed IPv6 literal in `{url}`"));
        };
        let host = &authority[..close + 2];
        let tail = &authority[close + 2..];
        let port = parse_port(tail, url)?;
        (host.to_string(), port.unwrap_or(80))
    } else if authority.contains(':') {
        // `host:port` — a colon outside brackets is the port separator.
        let Some((host, port)) = authority.rsplit_once(':') else {
            return Err(format!("malformed authority in `{url}`"));
        };
        if host.is_empty() {
            return Err(format!("URL has an empty host: `{url}`"));
        }
        let port = parse_port(&format!(":{port}"), url)?.ok_or_else(|| {
            format!("URL is missing a port after `:`: `{url}`")
        })?;
        (host.to_string(), port)
    } else {
        (authority.to_string(), 80)
    };
    if port == 0 {
        return Err(format!("port 0 is not a remote port: `{url}`"));
    }
    // The fragment is not sent; a missing path is `/`.
    let path_query = remainder.split('#').next().unwrap_or("");
    let target = if path_query.is_empty() {
        "/".to_string()
    } else if path_query.starts_with('?') {
        format!("/{path_query}")
    } else {
        path_query.to_string()
    };
    let connect_host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(&host)
        .to_string();
    Ok(Url {
        host,
        connect_host,
        port,
        target,
    })
}

/// Parse the `:port` tail of an authority: absent is `None`, otherwise the
/// decimal port the spec describes.
fn parse_port(tail: &str, url: &str) -> Result<Option<u16>, String> {
    if tail.is_empty() {
        return Ok(None);
    }
    let digits = tail.strip_prefix(':').unwrap_or("");
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("malformed port in `{url}`"));
    }
    let port: u32 = digits
        .parse()
        .map_err(|_| format!("malformed port in `{url}`"))?;
    u16::try_from(port)
        .map(Some)
        .map_err(|_| format!("malformed port in `{url}`"))
}

// ──────────────────────────────────────────────────────────────────
//  Client
// ──────────────────────────────────────────────────────────────────

/// The lifetime budget of one client call: measured from the call's start
/// and covering name resolution, the connection, the request write, and the
/// whole response. `None` waits without limit.
struct Budget {
    deadline: Option<Instant>,
    ms: u64,
    what: String,
}

impl Budget {
    fn new(timeout: Option<u64>, what: String) -> Self {
        Self {
            deadline: timeout.map(|ms| Instant::now() + Duration::from_millis(ms)),
            ms: timeout.unwrap_or_default(),
            what,
        }
    }

    /// The time left on the budget — the expired-budget `Err` class when
    /// there is none left.
    fn remaining(&self) -> Result<Duration, String> {
        match self.deadline {
            None => Ok(Duration::from_secs(3600)),
            Some(dl) => {
                let now = Instant::now();
                if now >= dl {
                    Err(format!("`{}` timed out after {} ms", self.what, self.ms))
                } else {
                    Ok(dl - now)
                }
            }
        }
    }

    fn io_timeout(&self) -> Result<Option<Duration>, String> {
        self.remaining().map(Some)
    }
}

/// `std::http::fetch(url[, timeout])`: `GET` and read the whole answer —
/// exactly `request("GET", url, null, null, timeout)`.
pub fn fetch(url: &str, timeout: Option<u64>) -> Result<Response, String> {
    request("GET", url, None, None, timeout)
}

/// `std::http::request(method, url[, headers[, body[, timeout]]])`: any
/// method, with headers and a body. The caller's `Connection`,
/// `Content-Length`, and `Transfer-Encoding` are dropped — framing is
/// always the client's — and redirects are not followed.
pub fn request(
    method: &str,
    url: &str,
    headers: Option<Vec<Header>>,
    body: Option<&str>,
    timeout: Option<u64>,
) -> Result<Response, String> {
    let budget = Budget::new(timeout, url.to_string());
    if method.is_empty()
        || method
            .bytes()
            .any(|b| b <= 0x20 || b >= 0x7f)
    {
        return Err(format!("method contains a space, a control character, or a non-ASCII byte: `{method}`"));
    }
    let headers = headers.unwrap_or_default();
    for h in &headers {
        if h.name.is_empty()
            || h.name.bytes().any(|b| b < 0x20 || b == 0x7f)
            || h.value.bytes().any(|b| b < 0x20 && b != b'\t' || b == 0x7f)
        {
            return Err(format!(
                "header `{}: {}` contains CR, LF, or another control character",
                h.name, h.value
            ));
        }
    }
    let url_parts = parse_url(url)?;
    // Framing headers are the client's own; a caller's `Host` stands.
    let caller: Vec<Header> = headers
        .into_iter()
        .filter(|h| {
            !h.name.eq_ignore_ascii_case("connection")
                && !h.name.eq_ignore_ascii_case("content-length")
                && !h.name.eq_ignore_ascii_case("transfer-encoding")
        })
        .collect();
    let has_host = caller.iter().any(|h| h.name.eq_ignore_ascii_case("host"));
    let mut stream = connect(&url_parts.connect_host, url_parts.port, &budget)?;
    let mut head = format!("{} {} HTTP/1.1\r\n", method, url_parts.target);
    if !has_host {
        head.push_str(&format!("Host: {}\r\n", url_parts.host));
    }
    for h in &caller {
        head.push_str(&format!("{}: {}\r\n", h.name, h.value));
    }
    head.push_str("Connection: close\r\n");
    if let Some(b) = body {
        head.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    head.push_str("\r\n");
    stream
        .set_write_timeout(budget.io_timeout()?)
        .map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(budget.io_timeout()?)
        .map_err(|e| e.to_string())?;
    stream
        .write_all(head.as_bytes())
        .map_err(|e| map_io(e, &budget))?;
    if let Some(b) = body {
        stream
            .write_all(b.as_bytes())
            .map_err(|e| map_io(e, &budget))?;
    }
    stream.flush().map_err(|e| map_io(e, &budget))?;
    let is_head = method.eq_ignore_ascii_case("HEAD");
    read_response(stream, is_head, &budget)
}

fn connect(host: &str, port: u16, budget: &Budget) -> Result<TcpStream, String> {
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("cannot resolve `{host}`: {e}"))?
        .collect();
    let mut last = format!("cannot connect to `{host}:{port}`");
    for addr in addrs {
        let attempt = match budget.deadline {
            Some(_) => {
                let rem = budget.remaining()?;
                TcpStream::connect_timeout(&addr, rem)
            }
            None => TcpStream::connect(addr),
        };
        match attempt {
            Ok(s) => return Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                return Err(format!("`{}` timed out after {} ms", budget.what, budget.ms));
            }
            Err(e) => last = format!("cannot connect to `{host}:{port}`: {e}"),
        }
    }
    Err(last)
}

/// An I/O failure that may just be the budget running out.
fn map_io(e: std::io::Error, budget: &Budget) -> String {
    if matches!(
        e.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        format!("`{}` timed out after {} ms", budget.what, budget.ms)
    } else {
        format!("I/O failure on `{}`: {e}", budget.what)
    }
}

// ──────────────────────────────────────────────────────────────────
//  Shared reading: header block, body, chunks
// ──────────────────────────────────────────────────────────────────

/// The header block is at most 64 KiB; a body at most 16 MiB (spec,
/// "Client" and "Server").
const MAX_HEAD_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Read one CRLF- or LF-terminated line (without its terminator).
fn read_line<R: BufRead>(reader: &mut R, budget: &Budget) -> Result<String, String> {
    let mut bytes = Vec::new();
    loop {
        let avail = {
            let buf = reader.fill_buf().map_err(|e| map_io(e, budget))?;
            if buf.is_empty() {
                return if bytes.is_empty() {
                    Err("connection closed".to_string())
                } else {
                    Ok(String::from_utf8_lossy(&bytes).into_owned())
                };
            }
            match buf.iter().position(|&b| b == b'\n') {
                Some(i) => {
                    bytes.extend_from_slice(&buf[..=i]);
                    i + 1
                }
                None => {
                    bytes.extend_from_slice(buf);
                    buf.len()
                }
            }
        };
        reader.consume(avail);
        if bytes.last() == Some(&b'\n') {
            break;
        }
        budget.remaining()?;
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The header block as raw lines up to the blank one, bounded by
/// `MAX_HEAD_BYTES`.
fn read_head_lines<R: BufRead>(reader: &mut R, budget: &Budget) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let mut total = 0usize;
    loop {
        let line = read_line(reader, budget)?;
        total += line.len() + 2;
        if total > MAX_HEAD_BYTES {
            return Err("header block exceeds 64 KiB".to_string());
        }
        if line.is_empty() {
            return Ok(lines);
        }
        lines.push(line);
    }
}

/// Read exactly `n` more bytes.
fn read_exact_bytes<R: Read>(
    reader: &mut R,
    n: usize,
    budget: &Budget,
) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; n];
    let mut got = 0;
    while got < n {
        budget.remaining()?;
        let r = reader
            .read(&mut buf[got..])
            .map_err(|e| map_io(e, budget))?;
        if r == 0 {
            return Err("connection closed mid-body".to_string());
        }
        got += r;
    }
    Ok(buf)
}

/// Read to EOF, bounded by `MAX_BODY_BYTES`.
fn read_to_end_capped<R: Read>(reader: &mut R, budget: &Budget) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        budget.remaining()?;
        let r = reader.read(&mut chunk).map_err(|e| map_io(e, budget))?;
        if r == 0 {
            return Ok(buf);
        }
        buf.extend_from_slice(&chunk[..r]);
        if buf.len() > MAX_BODY_BYTES {
            return Err("body exceeds 16 MiB".to_string());
        }
    }
}

/// A `Transfer-Encoding: chunked` body: chunk sizes are hex, trailers are
/// discarded (spec, "Client").
fn read_chunked<R: BufRead>(reader: &mut R, budget: &Budget) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let line = read_line(reader, budget)?;
        let size_text = line.split(';').next().unwrap_or("").trim();
        let size = u64::from_str_radix(size_text, 16)
            .map_err(|_| format!("malformed chunk size `{size_text}`"))?;
        if size == 0 {
            // Trailers run until the blank line, then the body is complete.
            loop {
                let trailer = read_line(reader, budget)?;
                if trailer.is_empty() {
                    return Ok(body);
                }
            }
        }
        let size = size as usize;
        if body.len() + size > MAX_BODY_BYTES {
            return Err("body exceeds 16 MiB".to_string());
        }
        body.extend_from_slice(&read_exact_bytes(reader, size, budget)?);
        let _crlf = read_line(reader, budget)?;
    }
}

/// Parse `name: value` header lines (already split off the block).
fn parse_header_lines(lines: &[String]) -> Result<Vec<Header>, String> {
    let mut headers = Vec::with_capacity(lines.len());
    for line in lines {
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err("folded header line".to_string());
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(format!("header line without a colon: `{line}`"));
        };
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() || name.bytes().any(|b| b < 0x20 || b == 0x7f) {
            return Err(format!("malformed header name: `{line}`"));
        }
        headers.push(Header {
            name: name.to_string(),
            value: value.to_string(),
        });
    }
    Ok(headers)
}

/// Read one whole response (client side).
fn read_response(stream: TcpStream, is_head: bool, budget: &Budget) -> Result<Response, String> {
    let mut reader = BufReader::new(stream);
    let status_line = read_line(&mut reader, budget)?;
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    let code = parts.next().unwrap_or("");
    if version != "HTTP/1.0" && version != "HTTP/1.1" {
        return Err(format!("unsupported HTTP version `{version}`"));
    }
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("malformed status line: `{status_line}`"));
    }
    let status: i64 = code.parse().unwrap();
    if (100..=199).contains(&status) {
        return Err(format!("informational status `{status}` is a protocol violation"));
    }
    let lines = read_head_lines(&mut reader, budget)?;
    let headers = parse_header_lines(&lines)?;
    // Framing decides the body: chunked first, then Content-Length, then
    // close-delimited. `204`/`304` and `HEAD` never carry one.
    let body = if is_head || status == 204 || status == 304 {
        String::new()
    } else {
        let chunked = find_header(&headers, "transfer-encoding")
            .map(|h| h.value.eq_ignore_ascii_case("chunked"))
            .unwrap_or(false);
        let other_coding = find_header(&headers, "transfer-encoding")
            .map(|h| !h.value.eq_ignore_ascii_case("chunked"))
            .unwrap_or(false);
        if other_coding {
            return Err("unsupported transfer coding".to_string());
        }
        let bytes = if chunked {
            read_chunked(&mut reader, budget)?
        } else if let Some(cl) = find_header(&headers, "content-length") {
            let n: usize = cl
                .value
                .trim()
                .parse()
                .map_err(|_| format!("malformed Content-Length `{}`", cl.value))?;
            if n > MAX_BODY_BYTES {
                return Err("body exceeds 16 MiB".to_string());
            }
            read_exact_bytes(&mut reader, n, budget)?
        } else {
            read_to_end_capped(&mut reader, budget)?
        };
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Ok(Response {
        status,
        headers,
        body,
    })
}

// ──────────────────────────────────────────────────────────────────
//  Server
// ──────────────────────────────────────────────────────────────────

/// A bound, listening server (spec, "Server"): opaque — only `listen` issues
/// one. `serve` runs the whole server until accepting fails.
pub struct Server {
    listener: TcpListener,
    addr: String,
}

/// `std::http::listen(addr)`: bind and listen on the `host:port` grammar of
/// `net.md`, port `0` asking for an ephemeral port.
pub fn listen(addr: &str) -> Result<Server, String> {
    let (host, port) = split_host_port(addr)?;
    let listener = TcpListener::bind((host.as_str(), port))
        .map_err(|e| format!("cannot bind `{addr}`: {e}"))?;
    let local = listener
        .local_addr()
        .map_err(|e| format!("cannot read the bound address of `{addr}`: {e}"))?;
    Ok(Server {
        listener,
        addr: local.to_string(),
    })
}

/// Split `"host:port"` (bracketed IPv6 allowed), port decimal.
fn split_host_port(addr: &str) -> Result<(String, u16), String> {
    let (host, port_text) = if let Some(inner) = addr.strip_prefix('[') {
        let Some(close) = inner.find(']') else {
            return Err(format!("malformed IPv6 literal in `{addr}`"));
        };
        let host = &addr[..close + 2];
        let tail = &addr[close + 2..];
        let port = tail
            .strip_prefix(':')
            .ok_or_else(|| format!("`{addr}` is missing a port"))?;
        (host.to_string(), port.to_string())
    } else {
        let (host, port) = addr
            .rsplit_once(':')
            .ok_or_else(|| format!("`{addr}` is missing a port"))?;
        (host.to_string(), port.to_string())
    };
    if host.trim_start_matches('[').trim_end_matches(']').is_empty() {
        return Err(format!("`{addr}` has an empty host"));
    }
    if port_text.is_empty() || !port_text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("malformed port in `{addr}`"));
    }
    let port: u32 = port_text
        .parse()
        .map_err(|_| format!("malformed port in `{addr}`"))?;
    let port = u16::try_from(port).map_err(|_| format!("malformed port in `{addr}`"))?;
    Ok((host, port))
}

impl Server {
    /// `Server::addr()`: the bound `"ip:port"`, an ephemeral port resolved.
    pub fn addr(&self) -> Result<String, String> {
        Ok(self.addr.clone())
    }

    /// `Server::serve(handler)`: accept and answer one request per
    /// connection, in order. It blocks and returns only when accepting
    /// fails — as `Err`. A client that disconnects mid-response closes that
    /// connection; serving goes on.
    pub fn serve<F: Fn(&Request) -> Response>(&self, handler: F) -> Result<(), String> {
        loop {
            let (stream, peer) = match self.listener.accept() {
                Ok(pair) => pair,
                Err(e) => return Err(format!("cannot accept: {e}")),
            };
            let _ = handle_connection(stream, peer, &handler);
        }
    }
}

/// Answer status with no body and nothing but the status line — the error
/// answers of "Server" close the connection, the handler is not called.
/// Any request bytes still unread are drained first: closing over them would
/// reset the connection out from under the answer.
fn write_status_only(writer: &mut TcpStream, status: i64) {
    let _ = writer.write_all(
        format!("HTTP/1.1 {} {}\r\n\r\n", status, reason_phrase(status)).as_bytes(),
    );
    let _ = writer.flush();
    let _ = writer.shutdown(std::net::Shutdown::Write);
    let _ = writer.set_read_timeout(Some(Duration::from_millis(100)));
    let mut sink = [0u8; 1024];
    loop {
        match writer.read(&mut sink) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

/// One inbound request on one connection.
fn handle_connection<F: Fn(&Request) -> Response>(
    stream: TcpStream,
    peer: SocketAddr,
    handler: &F,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(300)))
        .map_err(|e| e.to_string())?;
    let mut writer = stream
        .try_clone()
        .map_err(|e| format!("cannot clone connection: {e}"))?;
    let budget = Budget::new(None, peer.to_string());
    let mut reader = BufReader::new(stream);
    let head = match read_head_lines(&mut reader, &budget) {
        Ok(head) => head,
        Err(e) => {
            if e.contains("64 KiB") {
                write_status_only(&mut writer, 431);
            } else {
                write_status_only(&mut writer, 400);
            }
            return Ok(());
        }
    };
    let mut lines = head.into_iter();
    let Some(request_line) = lines.next() else {
        write_status_only(&mut writer, 400);
        return Ok(());
    };
    // The request line is `METHOD SP target SP HTTP/1.1`.
    let mut parts = request_line.splitn(3, ' ');
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let version = parts.next().unwrap_or("").to_string();
    if method.is_empty() || target.is_empty() || version.is_empty() {
        write_status_only(&mut writer, 400);
        return Ok(());
    }
    if !version.starts_with("HTTP/") {
        write_status_only(&mut writer, 400);
        return Ok(());
    }
    if version != "HTTP/1.1" {
        write_status_only(&mut writer, 505);
        return Ok(());
    }
    let headers = match parse_header_lines(&lines.collect::<Vec<_>>()) {
        Ok(h) => h,
        Err(_) => {
            write_status_only(&mut writer, 400);
            return Ok(());
        }
    };
    // An HTTP/1.1 request carries exactly one non-empty `Host` without a
    // space.
    let hosts: Vec<&Header> = headers
        .iter()
        .filter(|h| h.name.eq_ignore_ascii_case("host"))
        .collect();
    if hosts.len() != 1 || hosts[0].value.is_empty() || hosts[0].value.contains(' ') {
        write_status_only(&mut writer, 400);
        return Ok(());
    }
    // `Expect: 100-continue` gets the interim answer before the body; any
    // other Expect is refused. `Transfer-Encoding` is unsupported.
    if find_header(&headers, "transfer-encoding").is_some() {
        write_status_only(&mut writer, 501);
        return Ok(());
    }
    let content_length = {
        let cls: Vec<&Header> = headers
            .iter()
            .filter(|h| h.name.eq_ignore_ascii_case("content-length"))
            .collect();
        match cls.len() {
            0 => None,
            1 => {
                let text = cls[0].value.trim();
                if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
                    write_status_only(&mut writer, 400);
                    return Ok(());
                }
                match text.parse::<usize>() {
                    Ok(n) if n <= MAX_BODY_BYTES => Some(n),
                    Ok(_) => {
                        write_status_only(&mut writer, 413);
                        return Ok(());
                    }
                    Err(_) => {
                        write_status_only(&mut writer, 400);
                        return Ok(());
                    }
                }
            }
            _ => {
                // Repeated or conflicting lengths are malformed.
                write_status_only(&mut writer, 400);
                return Ok(());
            }
        }
    };
    if let Some(expect) = find_header(&headers, "expect") {
        if expect.value.eq_ignore_ascii_case("100-continue") {
            if content_length.is_some() {
                let _ = writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
                let _ = writer.flush();
            }
        } else {
            write_status_only(&mut writer, 417);
            return Ok(());
        }
    }
    // The body runs only when `Content-Length` says so.
    let body_bytes = match content_length {
        Some(n) => match read_exact_bytes(&mut reader, n, &budget) {
            Ok(b) => b,
            Err(_) => {
                write_status_only(&mut writer, 400);
                return Ok(());
            }
        },
        None => Vec::new(),
    };
    let body = String::from_utf8_lossy(&body_bytes).into_owned();
    // `form` is decoded only for a form body; any other media type leaves it
    // empty.
    let form = {
        let is_form = find_header(&headers, "content-type")
            .map(|h| {
                let media = h.value.split(';').next().unwrap_or("").trim();
                media.eq_ignore_ascii_case("application/x-www-form-urlencoded")
            })
            .unwrap_or(false);
        if is_form {
            match decode_form(&body) {
                Ok(form) => form,
                Err(_) => {
                    write_status_only(&mut writer, 400);
                    return Ok(());
                }
            }
        } else {
            Vec::new()
        }
    };
    // `path` and `query` split at the first `?`; the target must start with
    // `/`, and percent escapes decode as the URL section describes.
    if !target.starts_with('/') {
        write_status_only(&mut writer, 400);
        return Ok(());
    }
    let (raw_path, raw_query) = match target.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (target.as_str(), None),
    };
    let path = match percent_decode(raw_path, false) {
        Ok(p) => p,
        Err(_) => {
            write_status_only(&mut writer, 400);
            return Ok(());
        }
    };
    let query = match raw_query {
        None | Some("") => Vec::new(),
        Some(q) => match decode_query(q) {
            Ok(pairs) => pairs,
            Err(_) => {
                write_status_only(&mut writer, 400);
                return Ok(());
            }
        },
    };
    let request = Request {
        method,
        path,
        query,
        form,
        headers,
        body,
        version,
        remote_addr: peer.to_string(),
    };
    let mut response = handler(&request);
    // A status outside `200 …= 599` is sent as `500`.
    if !(200..=599).contains(&response.status) {
        response.status = 500;
    }
    write_response(
        &mut writer,
        &request,
        response.status,
        &response.headers,
        &response.body,
    );
    Ok(())
}

/// Decode a form body: parts split at `&` (empty parts ignored), each at its
/// first `=` (a part without one has the empty value), `+` becoming a space
/// before `%XX` is decoded. A malformed escape is `Err`.
fn decode_form(body: &str) -> Result<Vec<Header>, String> {
    if body.is_empty() {
        return Ok(Vec::new());
    }
    let mut parts = Vec::new();
    for part in body.split('&') {
        if part.is_empty() {
            continue;
        }
        let (name, value) = match part.split_once('=') {
            Some((n, v)) => (n, v),
            None => (part, ""),
        };
        parts.push(Header {
            name: percent_decode(name, true)?,
            value: percent_decode(value, true)?,
        });
    }
    Ok(parts)
}

/// Decode a query string: parts split at `&` (empty parts ignored), each at
/// its first `=`, `%XX` decoded with `+` staying `+` (URL syntax, not form
/// encoding).
fn decode_query(raw: &str) -> Result<Vec<Header>, String> {
    let mut pairs = Vec::new();
    for part in raw.split('&') {
        if part.is_empty() {
            continue;
        }
        let (name, value) = match part.split_once('=') {
            Some((n, v)) => (n, v),
            None => (part, ""),
        };
        pairs.push(Header {
            name: percent_decode(name, false)?,
            value: percent_decode(value, false)?,
        });
    }
    Ok(pairs)
}

/// Write one response: the handler's headers in order minus the framing
/// three, then `Content-Length`, `Connection: close`, and a `Date` the host
/// supplies unless the handler did. `HEAD` gets the headers but no body;
/// `204`/`304` write no body and no `Content-Length`.
fn write_response(
    writer: &mut TcpStream,
    request: &Request,
    status: i64,
    headers: &[Header],
    body: &str,
) {
    let mut out = format!("HTTP/1.1 {} {}\r\n", status, reason_phrase(status));
    let mut has_date = false;
    for h in headers {
        let lower = h.name.to_ascii_lowercase();
        if lower == "content-length" || lower == "connection" || lower == "transfer-encoding" {
            continue;
        }
        if lower == "date" {
            has_date = true;
        }
        out.push_str(&format!("{}: {}\r\n", h.name, h.value));
    }
    if !has_date {
        out.push_str(&format!("Date: {}\r\n", http_date(SystemTime::now())));
    }
    let no_body = status == 204 || status == 304;
    if !no_body {
        out.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    out.push_str("Connection: close\r\n\r\n");
    let is_head = request.method.eq_ignore_ascii_case("HEAD");
    if !no_body && !is_head {
        out.push_str(body);
    }
    let _ = writer.write_all(out.as_bytes());
    let _ = writer.flush();
}

/// The reason phrase is the host's affair — a client ignores it. Common
/// statuses get their conventional phrase; anything else is bare.
fn reason_phrase(status: i64) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Content Too Large",
        414 => "URI Too Long",
        417 => "Expectation Failed",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        505 => "HTTP Version Not Supported",
        _ => "OK",
    }
}

/// An IMF-fixdate from the host clock (`Sun, 06 Nov 1994 08:49:37 GMT`).
fn http_date(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let sod = secs % 86_400;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(m - 1) as usize];
    let weekday = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][days.rem_euclid(7) as usize];
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        weekday,
        d,
        month,
        y,
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}
