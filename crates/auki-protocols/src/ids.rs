//! Authenticated Auki Domain application protocol identifiers.
//!
//! These constants name the retained SDK protocols. They are deliberately
//! separate from the retired unauthenticated IDs. They are not an allow-list:
//! `auki_p2p::ApplicationProtocol` remains the extension boundary for
//! additional authenticated protocols.

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
        ApplicationProtocol::new("/posemesh/example/v1")
            .expect("product-owned namespaces use the generic authenticated boundary");
    }
}
