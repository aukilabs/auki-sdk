//! Authenticated Auki Domain application protocol identifiers.
//!
//! These constants name the retained SDK protocols. They are deliberately
//! separate from the legacy IDs that the old runtime still serves during the
//! migration. They are not an allow-list: [`auki_p2p::ApplicationProtocol`]
//! remains the extension boundary for additional authenticated protocols.

/// Participant information without Manager-era fields.
pub const INFO_V1_0_0: &str = "/auki/auth/1/info/1.0.0";

/// Resource catalog payload version 0.2.0.
pub const RESOURCES_V0_2_0: &str = "/auki/auth/1/resources/0.2.0";

/// Resource catalog payload version 0.3.0.
pub const RESOURCES_V0_3_0: &str = "/auki/auth/1/resources/0.3.0";

/// Resource catalog payload version 0.4.0.
pub const RESOURCES_V0_4_0: &str = "/auki/auth/1/resources/0.4.0";

/// Registry Get-only payload version 0.2.0.
pub const REGISTRIES_V0_2_0: &str = "/auki/auth/1/registries/0.2.0";

/// Registry list-and-fetch payload version 0.3.0.
pub const REGISTRIES_V0_3_0: &str = "/auki/auth/1/registries/0.3.0";

/// Content-addressed blob payload version 0.1.0.
pub const BLOBS_V0_1_0: &str = "/auki/auth/1/blobs/0.1.0";

/// Live typed-message payload version 0.1.0.
pub const MESSAGE_V0_1_0: &str = "/auki/auth/1/message/0.1.0";

/// Native typed-stream payload version 0.2.0.
pub const STREAM_V0_2_0: &str = "/auki/auth/1/stream/0.2.0";

/// Every retained SDK application protocol under the authenticated transport.
pub const AUTHENTICATED_PROTOCOL_IDS: [&str; 9] = [
    INFO_V1_0_0,
    RESOURCES_V0_2_0,
    RESOURCES_V0_3_0,
    RESOURCES_V0_4_0,
    REGISTRIES_V0_2_0,
    REGISTRIES_V0_3_0,
    BLOBS_V0_1_0,
    MESSAGE_V0_1_0,
    STREAM_V0_2_0,
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use auki_p2p::ApplicationProtocol;

    use super::*;

    const LEGACY_PROTOCOL_IDS: [&str; 9] = [
        "/auki/info/0.0.1",
        "/auki/resources/0.2.0",
        "/auki/resources/0.3.0",
        "/auki/resources/0.4.0",
        "/auki/registries/0.2.0",
        "/auki/registries/0.3.0",
        "/auki/blobs/0.1.0",
        "/auki/message/0.1.0",
        "/auki/stream/0.2.0",
    ];

    #[test]
    fn d11_authenticated_protocol_ids_are_exact_unique_and_transport_valid() {
        assert_eq!(
            AUTHENTICATED_PROTOCOL_IDS,
            [
                "/auki/auth/1/info/1.0.0",
                "/auki/auth/1/resources/0.2.0",
                "/auki/auth/1/resources/0.3.0",
                "/auki/auth/1/resources/0.4.0",
                "/auki/auth/1/registries/0.2.0",
                "/auki/auth/1/registries/0.3.0",
                "/auki/auth/1/blobs/0.1.0",
                "/auki/auth/1/message/0.1.0",
                "/auki/auth/1/stream/0.2.0",
            ]
        );
        assert_eq!(
            AUTHENTICATED_PROTOCOL_IDS
                .iter()
                .copied()
                .collect::<HashSet<_>>()
                .len(),
            AUTHENTICATED_PROTOCOL_IDS.len()
        );
        for protocol in AUTHENTICATED_PROTOCOL_IDS {
            ApplicationProtocol::new(protocol).expect("locked D11 ID must be valid");
        }
    }

    #[test]
    fn legacy_ids_are_absent_from_authenticated_constants() {
        for legacy_id in LEGACY_PROTOCOL_IDS {
            assert!(!AUTHENTICATED_PROTOCOL_IDS.contains(&legacy_id));
            assert!(
                ApplicationProtocol::new(legacy_id).is_err(),
                "legacy protocol must be rejected before negotiation: {legacy_id}"
            );
        }
    }

    #[test]
    fn protocol_extension_boundary_is_not_a_closed_sdk_allow_list() {
        ApplicationProtocol::new("/auki-p2p/example/1.0.0")
            .expect("third-party protocols use the generic authenticated boundary");
    }

    #[cfg(feature = "swarm")]
    #[tokio::test(flavor = "multi_thread")]
    async fn legacy_peer_negotiation_is_unsupported_by_a_d11_only_node() {
        use std::time::Duration;

        use auki_p2p::{DdsTokenVerifier, Identity, Node, SessionRequirements};
        use futures::StreamExt;
        use libp2p::{StreamProtocol, multiaddr::Protocol, swarm::SwarmEvent};
        use libp2p_stream::OpenStreamError;

        use crate::{
            PeerIdentity,
            swarm::{SwarmConfig, build_swarm},
        };

        const TEST_DDS_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;
        const DOMAIN_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

        let server = Node::start(
            Identity::generate(),
            DdsTokenVerifier::from_es256_pem(TEST_DDS_PUBLIC_KEY).unwrap(),
            ["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
        )
        .unwrap();
        let _d11_registration = server
            .accept(
                ApplicationProtocol::new(INFO_V1_0_0).unwrap(),
                SessionRequirements::new(DOMAIN_ID).unwrap(),
            )
            .unwrap();
        let server_peer_id = server.peer_id();
        let server_address = server
            .first_listen_address()
            .await
            .unwrap()
            .with(Protocol::P2p(server_peer_id));

        let client_identity = PeerIdentity::from_seed(&[0x61; 32]);
        let mut client = build_swarm(
            &client_identity,
            SwarmConfig {
                listen_addresses: Vec::new(),
                agent_version: "legacy-negotiation-test/0".into(),
                enable_relay_server: false,
            },
        )
        .unwrap();
        let mut control = client.behaviour().stream.new_control();
        client.dial(server_address).unwrap();

        let (connected_sender, connected_receiver) = tokio::sync::oneshot::channel();
        let client_driver = tokio::spawn(async move {
            let mut connected_sender = Some(connected_sender);
            while let Some(event) = client.next().await {
                if matches!(
                    event,
                    SwarmEvent::ConnectionEstablished { peer_id, .. }
                        if peer_id == server_peer_id
                ) {
                    if let Some(sender) = connected_sender.take() {
                        let _ = sender.send(());
                    }
                }
            }
        });
        tokio::time::timeout(Duration::from_secs(5), connected_receiver)
            .await
            .expect("legacy test client did not connect")
            .expect("legacy client swarm stopped before connecting");

        for legacy_id in LEGACY_PROTOCOL_IDS {
            let error = tokio::time::timeout(
                Duration::from_secs(5),
                control.open_stream(server_peer_id, StreamProtocol::new(legacy_id)),
            )
            .await
            .expect("legacy protocol negotiation timed out")
            .expect_err("D11-only node negotiated a legacy protocol");
            assert!(
                matches!(error, OpenStreamError::UnsupportedProtocol(ref protocol) if protocol.as_ref() == legacy_id),
                "unexpected negotiation failure for {legacy_id}: {error}"
            );
        }

        client_driver.abort();
        let _ = client_driver.await;
        server.shutdown().await.unwrap();
    }
}
