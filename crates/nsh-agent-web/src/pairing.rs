//! Which page the agent drives: the one that presents the session's key
//! from this machine, while no other does.
//!
//! A tab opened by hand keeps its key in session storage, so that a reload
//! stays paired. Duplicating the tab copies session storage, so the copy
//! presents the same key. One key drives one tab: a page presenting it
//! pairs only while no page that paired is still connected, and is refused
//! otherwise (it runs as an ordinary development page, and `page.js` shows
//! the refusal and forgets the key). The key is usable again once the
//! paired page's connection has closed. A reload closes the old document's
//! connection before the new document connects, so a reload pairs again;
//! a duplicate comes while the tab it copies is still connected, so it is
//! refused.
//!
//! The dev server says whether a page that takes steps is connected
//! ([`DevServer::has_tester`]); in `nsh run` only paired pages take them.
//! Between a page's acceptance and its connection counting as connected
//! there is a moment (one message sent on loopback) in which a second page
//! presenting the key would pair too; a duplicate a person opens comes long
//! after.

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use nsh_dev::DevServer;
use serde_json::Value;

/// The session's key and its server, while a [`Pairing`] lives.
static PAIRING: Mutex<Option<(String, Arc<DevServer>)>> = Mutex::new(None);

/// Pages pair with the key while it lives; none after.
pub(crate) struct Pairing(());

impl Drop for Pairing {
    fn drop(&mut self) {
        // The server is let go here, on the thread that drops the agent:
        // `accept` only borrows it, so never on one of the server's own.
        let pairing = PAIRING.lock().unwrap_or_else(|e| e.into_inner()).take();
        drop(pairing);
    }
}

/// Pairs `server`'s pages by `key` until the returned [`Pairing`] drops.
pub(crate) fn install(server: &Arc<DevServer>, key: String) -> Pairing {
    *PAIRING.lock().unwrap_or_else(|e| e.into_inner()) = Some((key, server.clone()));
    server.set_observer_hook(accept);
    Pairing(())
}

fn matches(expected: &str, key: &str, peer: SocketAddr) -> bool {
    let same = expected.len() == key.len()
        && expected
            .bytes()
            .zip(key.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0;
    same && peer.ip().is_loopback()
}

fn accept(peer: SocketAddr, extra: &Value) -> bool {
    let Some(key) = extra.as_str() else {
        return false;
    };
    PAIRING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|(expected, server)| matches(expected, key, peer) && !server.has_tester())
}

#[cfg(test)]
mod tests {
    use std::{
        net::TcpStream,
        sync::Arc,
        time::{Duration, Instant},
    };

    use nsh_dev::{DevServer, protocol::PROTOCOL_VERSION};
    use serde_json::{Value, json};
    use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

    #[test]
    fn only_matching_keys_from_loopback_pair() {
        for (key, peer, accepted) in [
            ("abcd", "127.0.0.1:7", true),
            ("abcd", "[::1]:7", true),
            ("abce", "127.0.0.1:7", false),
            ("abc", "127.0.0.1:7", false),
            ("abcd", "192.0.2.1:7", false),
        ] {
            assert_eq!(super::matches("abcd", key, peer.parse().unwrap()), accepted);
        }
    }

    type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

    /// A page on `server` presenting `key`: its connection, and whether it
    /// paired (the server's hello says it observes).
    fn page(server: &DevServer, key: &str) -> (Socket, bool) {
        let url = format!("ws://{}/ws", server.local_addr());
        let (mut socket, _) = tungstenite::connect(url).unwrap();
        let hello = json!({
            "type": "hello",
            "protocol": PROTOCOL_VERSION,
            "bytecode_version": "",
            "device": "Web (test)",
            "platform": "web",
            "tests": true,
            "extra": key,
        });
        socket.send(Message::text(hello.to_string())).unwrap();
        let reply = loop {
            if let Message::Text(text) = socket.read().unwrap() {
                break serde_json::from_str::<Value>(text.as_str()).unwrap();
            }
        };
        assert_eq!(reply["type"], "hello", "{reply}");
        (socket, reply["observer"] == true)
    }

    fn until(what: &str, done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn one_key_drives_one_page_and_a_reload_pairs_again() {
        let server = Arc::new(
            DevServer::start("127.0.0.1:0".parse().unwrap(), "test", None, |_| {}).unwrap(),
        );
        let key = "ab".repeat(16);
        let pairing = super::install(&server, key.clone());

        let (mut first, paired) = page(&server, &key);
        assert!(paired, "the first page pairs");
        until("the first page connects", || server.has_tester());

        // A duplicate presents the same key while the first is connected.
        let (duplicate, paired) = page(&server, &key);
        assert!(!paired, "a second page with the key is refused");
        assert!(server.has_tester());

        // A reload: the old document's connection closes, then the new one
        // connects.
        first.close(None).unwrap();
        while first.read().is_ok() {}
        until("the first page disconnects", || !server.has_tester());
        let (reloaded, paired) = page(&server, &key);
        assert!(paired, "a reload pairs again");
        until("the reloaded page connects", || server.has_tester());
        let (_another, paired) = page(&server, &key);
        assert!(!paired, "the reloaded page keeps the pairing");

        drop((duplicate, reloaded));
        drop(pairing);
        let (_late, paired) = page(&server, &key);
        assert!(!paired, "no page pairs once the agent is gone");
    }
}
