//! Bounded, allowlisted connection diagnostics. Never format an underlying
//! error: its message can contain remote data, addresses or credentials.

use std::{error::Error, io};

use libp2p::{core::ConnectedPoint, multiaddr::Protocol, swarm::ConnectionError};

#[derive(Debug, Default)]
struct IoDetails {
    kinds: Vec<io::ErrorKind>,
    os_codes: Vec<i32>,
    layers: Vec<&'static str>,
    websocket_error: Option<&'static str>,
    truncated: bool,
}

fn io_details(error: &(dyn Error + 'static)) -> IoDetails {
    let mut details = IoDetails::default();
    let mut current = Some(error);
    // Also bounds buggy/cyclic Error::source implementations.
    for _ in 0..16 {
        let Some(error) = current else { return details };
        if let Some(error) = error.downcast_ref::<io::Error>() {
            details.layers.push("io");
            details.kinds.push(error.kind());
            if let Some(code) = error.raw_os_error() {
                details.os_codes.push(code);
            }
            // io::Error::source() skips its stored error's outermost type.
            current = error.get_ref().map(|inner| inner as &dyn Error);
            continue;
        }
        if error.is::<libp2p::yamux::Error>() {
            details.layers.push("yamux");
        } else if let Some(error) = error.downcast_ref::<libp2p::noise::Error>() {
            details.layers.push("noise");
            if let libp2p::noise::Error::Io(inner) = error {
                current = Some(inner);
                continue;
            }
        } else {
            details.layers.push("other");
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(error) = error.downcast_ref::<soketto::connection::Error>() {
            *details.layers.last_mut().expect("layer recorded") = "websocket";
            details.websocket_error = Some(match error {
                soketto::connection::Error::Io(_) => "io",
                soketto::connection::Error::Codec(_) => "codec",
                soketto::connection::Error::Extension(_) => "extension",
                soketto::connection::Error::UnexpectedOpCode(_) => "unexpected_opcode",
                soketto::connection::Error::Utf8(_) => "invalid_utf8",
                soketto::connection::Error::MessageTooLarge { .. } => "message_too_large",
                // Does not distinguish an orderly close from an EOF.
                soketto::connection::Error::Closed => "closed",
                _ => "other",
            });
        }
        current = error.source();
    }
    details.truncated = current.is_some();
    details
}

pub(crate) fn log_connection_closed(
    local_peer_id: &libp2p::PeerId,
    remote_peer_id: &libp2p::PeerId,
    connection_id: libp2p::swarm::ConnectionId,
    endpoint: &ConnectedPoint,
    cause: Option<&ConnectionError>,
) {
    let reason = match cause {
        None => "local_close",
        Some(ConnectionError::KeepAliveTimeout) => "keep_alive_timeout",
        Some(ConnectionError::IO(_)) => "io_error",
    };
    let io = match cause {
        Some(ConnectionError::IO(error)) => Some(error),
        _ => None,
    };
    let details = io.map(|error| io_details(error));
    let address = endpoint.get_remote_address();
    let transport = if address.iter().any(|p| matches!(p, Protocol::Wss(_))) {
        "wss"
    } else if address.iter().any(|p| matches!(p, Protocol::Ws(_))) {
        "ws"
    } else if address.iter().any(|p| matches!(p, Protocol::Tcp(_))) {
        "tcp"
    } else {
        "other"
    };
    tracing::info!(target: "auki_p2p::relay_recovery",
        %local_peer_id, %remote_peer_id, %connection_id,
        relayed = endpoint.is_relayed(), transport, reason,
        io_kind = ?io.map(io::Error::kind),
        io_chain = ?details.as_ref().map(|d| &d.kinds),
        os_codes = ?details.as_ref().map(|d| &d.os_codes),
        error_layers = ?details.as_ref().map(|d| &d.layers),
        websocket_error = ?details.as_ref().and_then(|d| d.websocket_error),
        diagnostics_truncated = details.as_ref().is_some_and(|d| d.truncated),
        "connection closed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Opaque;
    impl std::fmt::Display for Opaque {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("must never format a remote error body")
        }
    }
    impl Error for Opaque {}

    #[test]
    fn nested_io_preserves_inner_kind_without_formatting_error_bodies() {
        let error = io::Error::other(io::Error::new(io::ErrorKind::ConnectionReset, Opaque));
        let details = io_details(&error);
        assert_eq!(
            details.kinds,
            [io::ErrorKind::Other, io::ErrorKind::ConnectionReset]
        );
        assert_eq!(details.layers, ["io", "io", "other"]);
        assert!(!details.truncated);
        assert!(!format!("{details:?}").contains("Opaque"));
    }

    #[derive(Debug)]
    struct Cycle;
    impl std::fmt::Display for Cycle {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            unreachable!()
        }
    }
    impl Error for Cycle {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(self)
        }
    }

    #[test]
    fn error_chain_has_a_hard_bound() {
        let details = io_details(&Cycle);
        assert!(details.truncated);
        assert_eq!(details.layers.len(), 16);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn websocket_error_and_nested_io_are_classified_by_type() {
        let reset = io::Error::new(io::ErrorKind::ConnectionReset, Opaque);
        let error = io::Error::other(soketto::connection::Error::Io(reset));
        let details = io_details(&error);
        assert_eq!(details.websocket_error, Some("io"));
        assert_eq!(
            details.kinds,
            [io::ErrorKind::Other, io::ErrorKind::ConnectionReset]
        );
        for (error, label) in [
            (soketto::connection::Error::Closed, "closed"),
            (
                soketto::connection::Error::MessageTooLarge {
                    current: 42,
                    maximum: 10,
                },
                "message_too_large",
            ),
        ] {
            let details = io_details(&io::Error::other(error));
            assert_eq!(details.websocket_error, Some(label));
        }
    }
}
