//! libp2p stream protocol identifiers for the v1 runtime.

use libp2p::StreamProtocol;

/// V1 lifecycle protocol id.
pub const CLUSTER_LIFECYCLE_PROTOCOL_ID: &str = "/auki/cluster-lifecycle/0.0.1";
/// V1 offer-catalog protocol id.
pub const OFFER_CATALOG_PROTOCOL_ID: &str = "/auki/offer-catalog/0.0.1";
/// V1 Get protocol id.
pub const GET_PROTOCOL_ID: &str = "/auki/get/0.0.1";
/// V1 Subscribe protocol id.
pub const SUBSCRIBE_PROTOCOL_ID: &str = "/auki/subscribe/0.0.1";

/// Return the v1 lifecycle protocol as a libp2p stream protocol.
pub fn cluster_lifecycle_protocol() -> StreamProtocol {
    StreamProtocol::new(CLUSTER_LIFECYCLE_PROTOCOL_ID)
}

/// Return the v1 offer-catalog protocol as a libp2p stream protocol.
pub fn offer_catalog_protocol() -> StreamProtocol {
    StreamProtocol::new(OFFER_CATALOG_PROTOCOL_ID)
}

/// Return the v1 Get protocol as a libp2p stream protocol.
pub fn get_protocol() -> StreamProtocol {
    StreamProtocol::new(GET_PROTOCOL_ID)
}

/// Return the v1 Subscribe protocol as a libp2p stream protocol.
pub fn subscribe_protocol() -> StreamProtocol {
    StreamProtocol::new(SUBSCRIBE_PROTOCOL_ID)
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_protocol::v1::{
        get, handshake,
        offer::{self, OFFER_CATALOG_PROTOCOL_ID as PROTOCOL_OFFER_CATALOG_PROTOCOL_ID},
        subscribe,
    };

    #[test]
    fn protocol_ids_match_auki_protocol() {
        assert_eq!(
            CLUSTER_LIFECYCLE_PROTOCOL_ID,
            "/auki/cluster-lifecycle/0.0.1"
        );
        assert_eq!(handshake::CLUSTER_LIFECYCLE_V1, "auki.cluster_lifecycle.v1");
        assert_eq!(
            OFFER_CATALOG_PROTOCOL_ID,
            PROTOCOL_OFFER_CATALOG_PROTOCOL_ID
        );
        assert_eq!(GET_PROTOCOL_ID, get::GET_PROTOCOL_ID);
        assert_eq!(SUBSCRIBE_PROTOCOL_ID, subscribe::SUBSCRIBE_PROTOCOL_ID);
        assert_eq!(
            offer_catalog_protocol().to_string(),
            offer::OFFER_CATALOG_PROTOCOL_ID
        );
    }
}
