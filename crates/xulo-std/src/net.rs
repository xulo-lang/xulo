//! The `net` namespace: TCP sockets over IPv4/IPv6 (spec
//! `memory-and-runtime.md` — host-supplied I/O, implementation-first).
//!
//! Sockets are **opaque numeric handles** (Model A): this module keeps the
//! host-side `handle -> socket` registry, and every member validates its
//! handle first, so a foreign, closed, or never-opened number is a clean
//! `Err`, never a crash. The handle value itself is meaningless outside this
//! registry — reuse after `sock_close` fails because a fresh socket gets a
//! fresh handle.
//!
//! v1 is **synchronous and blocking**: `tcp_accept` and `sock_read` park the
//! calling thread (in the cooperative interpreter that freezes the whole
//! scheduler, exactly like `std::io::readLine` — there is no epoll in the
//! runtime yet). Tests and one-shot tools should handshake in program order:
//! `tcp_connect` completes through the listen backlog *before* `tcp_accept`
//! is called, so a single-threaded program never deadlocks.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::rc::Rc;

/// A registered socket: listeners accept, streams read and write. `Rc` keeps
/// a handle usable from both a `match` binding and the registry entry the
/// same op looks at.
enum Socket {
    Listener(TcpListener),
    Stream(Rc<RefCell<TcpStream>>),
}

thread_local! {
    static REGISTRY: RefCell<HashMap<i64, Socket>> = RefCell::new(HashMap::new());
    static NEXT_HANDLE: Cell<i64> = const { Cell::new(1) };
}

/// Reads and writes are capped so a garbage length cannot allocate
/// arbitrarily (`repeat`-style guard, same 1 Mi-element order of magnitude —
/// 16 MiB of text per read is far past any framed message this v1 targets).
const MAX_IO_BYTES: usize = 1 << 24;

fn lookup(handle: i64) -> Result<(i64, ()), String> {
    REGISTRY.with(|reg| {
        if reg.borrow().contains_key(&handle) {
            Ok((handle, ()))
        } else {
            Err(format!("socket handle {handle} is not open"))
        }
    })
}

fn register(socket: Socket) -> i64 {
    NEXT_HANDLE.with(|next| {
        let handle = next.get();
        next.set(handle + 1);
        REGISTRY.with(|reg| reg.borrow_mut().insert(handle, socket));
        handle
    })
}

/// Resolve `"host:port"` to socket addresses (DNS allowed; `127.0.0.1` and
/// `localhost` both work). An unresolvable name is an `Err`, not a panic.
fn resolve(addr: &str) -> Result<Vec<std::net::SocketAddr>, String> {
    addr.to_socket_addrs()
        .map(|iter| iter.collect())
        .map_err(|e| format!("cannot resolve `{addr}`: {e}"))
}

/// `std::net::tcp_listen(addr)`: create and bind a listener on `addr`
/// (`"127.0.0.1:8080"`; the listen backlog is the OS default, ~128) —
/// returns the listener handle. Port `0` asks the OS for an ephemeral port
/// (read it back with `sock_local_addr`).
pub fn tcp_listen(addr: &str) -> Result<i64, String> {
    let addrs = resolve(addr)?;
    let mut last = format!("cannot listen on `{addr}`");
    for candidate in addrs {
        match TcpListener::bind(candidate) {
            Ok(listener) => {
                let _ = listener.set_nonblocking(false);
                return Ok(register(Socket::Listener(listener)));
            }
            Err(e) => last = format!("cannot listen on `{addr}`: {e}"),
        }
    }
    Err(last)
}

/// `std::net::tcp_accept(listener)`: block until a client connects, then
/// return the new stream's handle. The operand must be a live listener.
/// The listener is taken out of the registry across the blocking accept so
/// the `RefCell` is never borrowed twice, then put straight back.
pub fn tcp_accept(listener: i64) -> Result<i64, String> {
    lookup(listener)?;
    let taken = REGISTRY.with(|reg| reg.borrow_mut().remove(&listener));
    let Some(socket) = taken else {
        return Err(format!("socket handle {listener} is not open"));
    };
    let l = match socket {
        Socket::Listener(l) => l,
        other => {
            REGISTRY.with(|reg| reg.borrow_mut().insert(listener, other));
            return Err(format!(
                "socket handle {listener} is a stream, not a listener"
            ));
        }
    };
    let accepted = l.accept();
    REGISTRY.with(|reg| reg.borrow_mut().insert(listener, Socket::Listener(l)));
    match accepted {
        Ok((stream, _peer)) => {
            let _ = stream.set_nonblocking(false);
            Ok(register(Socket::Stream(Rc::new(RefCell::new(stream)))))
        }
        Err(e) => Err(format!("accept failed: {e}")),
    }
}

/// `std::net::tcp_connect(addr)`: block until the connection completes (or
/// fails) — returns the stream's handle.
pub fn tcp_connect(addr: &str) -> Result<i64, String> {
    let addrs = resolve(addr)?;
    let mut last = format!("cannot connect to `{addr}`");
    for candidate in addrs {
        match TcpStream::connect(candidate) {
            Ok(stream) => {
                let _ = stream.set_nonblocking(false);
                return Ok(register(Socket::Stream(Rc::new(RefCell::new(stream)))));
            }
            Err(e) => last = format!("cannot connect to `{addr}`: {e}"),
        }
    }
    Err(last)
}

/// The stream a handle names, cloned out of the registry so the borrow is
/// not held across a blocking I/O call.
fn stream_of(handle: i64) -> Result<Rc<RefCell<TcpStream>>, String> {
    lookup(handle)?;
    REGISTRY.with(|reg| match reg.borrow().get(&handle) {
        Some(Socket::Stream(s)) => Ok(s.clone()),
        Some(Socket::Listener(_)) => Err(format!(
            "socket handle {handle} is a listener, not a stream"
        )),
        None => Err(format!("socket handle {handle} is not open")),
    })
}

/// `std::net::sock_read(handle, max)`: block for up to `max` bytes of input.
/// `Ok(null)` is end-of-stream (the peer closed — the same `null` read as
/// `io::readLine`'s EOF); `Ok(text)` may be shorter than `max`. TCP is
/// byte-oriented, so a multi-byte UTF-8 character split across two reads
/// surfaces as replacement bytes — frame messages when text matters.
pub fn sock_read(handle: i64, max: usize) -> Result<Option<String>, String> {
    // Argument domain first (same order as the FFI boundary): a zero or
    // oversized length is reported as itself, whatever the handle names.
    if max == 0 || max > MAX_IO_BYTES {
        return Err(format!(
            "read size must be between 1 and {MAX_IO_BYTES}, found {max}"
        ));
    }
    let stream = stream_of(handle)?;
    let mut buf = vec![0u8; max];
    match stream.borrow_mut().read(&mut buf) {
        Ok(0) => Ok(None),
        Ok(n) => {
            buf.truncate(n);
            Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
        }
        Err(e) => Err(format!("read failed: {e}")),
    }
}

/// `std::net::sock_write(handle, text)`: block until every byte of `text`
/// is handed to the kernel — returns the byte count (UTF-8 length).
pub fn sock_write(handle: i64, text: &str) -> Result<i64, String> {
    let stream = stream_of(handle)?;
    match stream.borrow_mut().write_all(text.as_bytes()) {
        Ok(()) => Ok(text.len() as i64),
        Err(e) => Err(format!("write failed: {e}")),
    }
}

/// `std::net::sock_shutdown(handle)`: half-close both directions so the peer
/// sees EOF while the handle stays open for `sock_close`.
pub fn sock_shutdown(handle: i64) -> Result<(), String> {
    let stream = stream_of(handle)?;
    match stream.borrow().shutdown(Shutdown::Both) {
        Ok(()) => Ok(()),
        Err(e) => Err(format!("shutdown failed: {e}")),
    }
}

/// `std::net::sock_close(handle)`: drop the socket from the registry (which
/// closes it) — listeners and streams alike. A second close is an `Err`
/// (the handle is already gone), and any later member call on it fails the
/// same validated way.
pub fn sock_close(handle: i64) -> Result<(), String> {
    lookup(handle)?;
    REGISTRY.with(|reg| reg.borrow_mut().remove(&handle));
    Ok(())
}

/// `std::net::sock_local_addr(handle)`: the bound address as `"ip:port"`
/// (an ephemeral port bound via `:0` reads back here).
pub fn sock_local_addr(handle: i64) -> Result<String, String> {
    lookup(handle)?;
    REGISTRY.with(|reg| match reg.borrow().get(&handle) {
        Some(Socket::Listener(l)) => l
            .local_addr()
            .map(|a| a.to_string())
            .map_err(|e| format!("local_addr failed: {e}")),
        Some(Socket::Stream(s)) => s
            .borrow()
            .local_addr()
            .map(|a| a.to_string())
            .map_err(|e| format!("local_addr failed: {e}")),
        None => Err(format!("socket handle {handle} is not open")),
    })
}

/// `std::net::sock_peer_addr(handle)`: the remote address of a connected
/// stream (`"ip:port"`); a listener has no peer, so it is an `Err`.
pub fn sock_peer_addr(handle: i64) -> Result<String, String> {
    let stream = stream_of(handle)?;
    stream
        .borrow()
        .peer_addr()
        .map(|a| a.to_string())
        .map_err(|e| format!("peer_addr failed: {e}"))
}
