use std::{
    net::SocketAddr,
    sync::{Mutex, OnceLock},
};

use nsh_dev::DevServer;
use serde_json::Value;

static KEY: OnceLock<Mutex<Option<String>>> = OnceLock::new();

pub(crate) fn install(server: &DevServer, key: String) {
    *KEY.get_or_init(Default::default)
        .lock()
        .expect("pairing lock") = Some(key);
    server.set_observer_hook(accept);
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
    KEY.get_or_init(Default::default)
        .lock()
        .expect("pairing lock")
        .as_ref()
        .is_some_and(|expected| matches(expected, key, peer))
}

#[cfg(test)]
mod tests {
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
}
