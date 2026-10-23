//! The `Net` namespace semantics in isolation — loopback TCP only
//! (127.0.0.1 with an OS-assigned port), no external network touched.
//! v1 is synchronous: every test handshakes in program order
//! (`listen` → `connect` → `accept`), so a connect completes through the
//! listen backlog before any `accept` blocks.

use xulo_std::net;

/// `tcp_listen("127.0.0.1:0")` plus the `"127.0.0.1:PORT"` string the
/// listener reports back (port 0 asks the OS for an ephemeral port).
fn listen_ephemeral() -> (i64, String) {
    let handle = net::tcp_listen("127.0.0.1:0").expect("loopback bind");
    let addr = net::sock_local_addr(handle).expect("local_addr");
    assert!(addr.starts_with("127.0.0.1:"), "bound addr: {addr}");
    assert!(!addr.ends_with(":0"), "port 0 should be assigned: {addr}");
    (handle, addr)
}

#[test]
fn loopback_echo_round_trip_in_program_order() {
    let (server, addr) = listen_ephemeral();
    let client = net::tcp_connect(&addr).expect("connect");
    let accepted = net::tcp_accept(server).expect("accept");

    // Client → server, then server → client: a request/response echo.
    assert_eq!(net::sock_write(client, "ping").unwrap(), 4);
    assert_eq!(net::sock_read(accepted, 64).unwrap(), Some("ping".into()));
    assert_eq!(net::sock_write(accepted, "pong").unwrap(), 4);
    assert_eq!(net::sock_read(client, 64).unwrap(), Some("pong".into()));

    // A shorter read than offered still returns exactly what arrived.
    assert_eq!(net::sock_write(client, "abcdef").unwrap(), 6);
    assert_eq!(net::sock_read(accepted, 3).unwrap(), Some("abc".into()));
    assert_eq!(net::sock_read(accepted, 3).unwrap(), Some("def".into()));

    net::sock_close(client).unwrap();
    net::sock_close(accepted).unwrap();
    net::sock_close(server).unwrap();
}

#[test]
fn utf8_byte_counts_and_lossy_chunk_reads() {
    let (server, addr) = listen_ephemeral();
    let client = net::tcp_connect(&addr).expect("connect");
    let accepted = net::tcp_accept(server).expect("accept");

    // `sock_write` reports bytes, not characters.
    let msg = "héllo, 世界";
    assert_eq!(net::sock_write(client, msg).unwrap(), msg.len() as i64);
    assert_eq!(net::sock_read(accepted, 1024).unwrap(), Some(msg.into()));

    // A read that slices a multi-byte character surfaces replacement bytes
    // (documented v1 behaviour: TCP is byte-oriented).
    assert_eq!(net::sock_write(client, "世界").unwrap(), 6);
    let chunk = net::sock_read(accepted, 2).unwrap().unwrap();
    // The wire slices the character in half (E4 B8); lossy decoding maps the
    // incomplete run to exactly one U+FFFD (three UTF-8 bytes in the string).
    assert_eq!(chunk, "\u{FFFD}");

    net::sock_close(client).unwrap();
    net::sock_close(accepted).unwrap();
    net::sock_close(server).unwrap();
}

#[test]
fn end_of_stream_is_ok_null_not_an_error() {
    let (server, addr) = listen_ephemeral();
    let client = net::tcp_connect(&addr).expect("connect");
    let accepted = net::tcp_accept(server).expect("accept");

    assert_eq!(net::sock_shutdown(client).unwrap(), ());
    assert_eq!(net::sock_read(accepted, 64).unwrap(), None);
    // EOF sticks: reading again stays `Ok(None)`.
    assert_eq!(net::sock_read(accepted, 64).unwrap(), None);

    net::sock_close(accepted).unwrap();
    net::sock_close(server).unwrap();
}

#[test]
fn closed_and_foreign_handles_are_clean_errors() {
    let (server, addr) = listen_ephemeral();
    let client = net::tcp_connect(&addr).expect("connect");
    let accepted = net::tcp_accept(server).expect("accept");

    // A handle the registry never issued (or a closed one) never crashes.
    let foreign = 999_999;
    assert!(net::sock_read(foreign, 8).unwrap_err().contains("not open"));
    assert!(
        net::sock_write(foreign, "x")
            .unwrap_err()
            .contains("not open")
    );
    assert!(net::sock_close(foreign).unwrap_err().contains("not open"));
    assert!(net::tcp_accept(foreign).unwrap_err().contains("not open"));
    assert!(
        net::sock_local_addr(foreign)
            .unwrap_err()
            .contains("not open")
    );
    assert!(
        net::sock_peer_addr(foreign)
            .unwrap_err()
            .contains("not open")
    );

    // Close is idempotent in intent but the second call sees no handle.
    net::sock_close(client).unwrap();
    assert!(net::sock_close(client).unwrap_err().contains("not open"));
    assert!(net::sock_read(client, 8).unwrap_err().contains("not open"));

    // Kind mismatches say so instead of misbehaving.
    assert!(
        net::sock_write(server, "x")
            .unwrap_err()
            .contains("listener, not a stream")
    );
    assert!(
        net::tcp_accept(accepted)
            .unwrap_err()
            .contains("stream, not a listener")
    );
    assert!(
        net::sock_peer_addr(server)
            .unwrap_err()
            .contains("listener, not a stream")
    );

    net::sock_close(accepted).unwrap();
    net::sock_close(server).unwrap();
}

#[test]
fn read_sizes_are_validated() {
    let (server, addr) = listen_ephemeral();
    let client = net::tcp_connect(&addr).expect("connect");
    let accepted = net::tcp_accept(server).expect("accept");

    let zero = net::sock_read(accepted, 0).unwrap_err();
    assert!(zero.contains("read size"), "{zero}");
    let huge = net::sock_read(accepted, 1 << 25).unwrap_err();
    assert!(huge.contains("read size"), "{huge}");

    net::sock_close(client).unwrap();
    net::sock_close(accepted).unwrap();
    net::sock_close(server).unwrap();
}

#[test]
fn unreachable_and_malformed_addresses_error_cleanly() {
    // Unresolvable host: a message, not a panic.
    let resolve = net::tcp_connect("no.such.host.invalid:80").unwrap_err();
    assert!(resolve.contains("cannot resolve"), "{resolve}");
    // Port out of range: reported, not wrapped around.
    let bad_port = net::tcp_listen("127.0.0.1:70000").unwrap_err();
    assert!(bad_port.contains("cannot"), "{bad_port}");
    // Connecting to a nobody: refused (or unreachable), as an Err.
    let refused = net::tcp_connect("127.0.0.1:9").unwrap_err();
    assert!(refused.contains("cannot connect"), "{refused}");
}

#[test]
fn handles_do_not_alias_across_lifetimes() {
    let (server, addr) = listen_ephemeral();
    let client = net::tcp_connect(&addr).expect("connect");
    let accepted = net::tcp_accept(server).expect("accept");

    // Each open gets a distinct handle, so stale numbers stay dead.
    assert_ne!(server, client);
    assert_ne!(client, accepted);
    net::sock_close(accepted).unwrap();
    // The next stream gets a fresh number — never the closed one's.
    let client2 = net::tcp_connect(&addr).expect("reconnect");
    assert_ne!(client2, accepted);
    assert!(
        net::sock_read(accepted, 8)
            .unwrap_err()
            .contains("not open")
    );

    net::sock_close(client).unwrap();
    net::sock_close(client2).unwrap();
    net::sock_close(server).unwrap();
}
