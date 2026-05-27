//! Networking substrate for the Auki SDK.
//!
//! The stable Rust API is re-exported from [`core`]. Native UniFFI and
//! JavaScript/WebAssembly bindings live in private adapter modules so the
//! binding shape can differ from the product logic.
//!
//! `swarm` feature: a native libp2p `Swarm` with TCP + QUIC + Noise + Yamux,
//! a [`libp2p-allow-block-list`] gate that enforces the cluster trust boundary
//! at the handshake layer, and a [`NetworkRuntime`] that drives the swarm
//! against a configurable allow-list.
//!
//! `wasm` feature: browser-facing identity and protocol helpers for the
//! generated JavaScript package. Browser transport is owned by that package via
//! jslibp2p, not by the Rust native runtime.

#[cfg(all(target_arch = "wasm32", feature = "uniffi"))]
compile_error!(
    "auki-network does not support UniFFI on wasm32; build wasm with --no-default-features --features wasm"
);

pub mod core;
pub use core::*;

pub mod signaled_address;
pub use signaled_address::*;

pub mod participant;
pub use participant::ParticipantInfo;

pub mod browser_probe_protocol;
pub use browser_probe_protocol::{
    BROWSER_PROBE_PROTOCOL, BrowserProbeRequest, BrowserProbeResponse,
};

#[cfg(all(feature = "uniffi", not(target_arch = "wasm32")))]
mod ffi;

#[cfg(all(feature = "uniffi", not(target_arch = "wasm32")))]
#[doc(hidden)]
pub use ffi::UniFfiTag;

#[cfg(all(feature = "uniffi", not(target_arch = "wasm32")))]
#[doc(hidden)]
pub use ffi::{
    NetworkError, PeerIdentity as BindingPeerIdentity, networking_capabilities,
    peer_derivation_label, peer_id_from_wallet_seed,
};

#[cfg(all(feature = "uniffi", feature = "swarm", not(target_arch = "wasm32")))]
#[doc(hidden)]
pub use ffi::{
    AukiNetworkRuntime, BindingAllowedPeer, BindingNetworkError, BindingProtocolResponse,
    BindingRuntimeEvent, BindingStreamEntry, BindingStreamRequest, BindingSwarmConfig,
    BindingUpdateReport,
};

#[cfg(all(
    feature = "uniffi",
    feature = "discovery_client",
    feature = "swarm",
    not(target_arch = "wasm32")
))]
#[doc(hidden)]
pub use ffi::{AukiDiscoveryClient, discovery_client};

#[cfg(all(
    feature = "uniffi",
    feature = "app_instance",
    feature = "swarm",
    not(target_arch = "wasm32")
))]
#[doc(hidden)]
pub use ffi::{app_instance_peer_id, derive_app_instance_json};

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
mod wasm;

#[cfg(feature = "browser_probe")]
pub mod browser_probe;

#[cfg(feature = "message_node")]
pub mod message_node;

#[cfg(feature = "swarm")]
pub mod swarm;

/// Re-export of `libp2p::Swarm` so downstream crates can name
/// `auki_network::Swarm<Behaviour>` without taking a direct `libp2p`
/// dep.
#[cfg(feature = "swarm")]
pub use libp2p::Swarm;

#[cfg(feature = "swarm")]
pub mod network_runtime;

#[cfg(feature = "swarm")]
pub mod stream_protocol;

#[cfg(feature = "swarm")]
pub mod stream_runtime;

#[cfg(feature = "swarm")]
pub mod join_protocol;

#[cfg(feature = "swarm")]
pub mod heartbeat_protocol;

#[cfg(feature = "swarm")]
pub mod membership_protocol;

#[cfg(feature = "swarm")]
pub mod diagnostic_protocol;

#[cfg(feature = "swarm")]
pub mod message_protocol;

#[cfg(feature = "swarm")]
pub mod info_protocol;

#[cfg(feature = "swarm")]
pub mod sensors_protocol;

#[cfg(feature = "swarm")]
pub mod resources_protocol;

#[cfg(feature = "swarm")]
pub mod registries_protocol;

#[cfg(feature = "swarm")]
pub use network_runtime::{
    AllowedPeer, BroadcastDiagnosticError, BroadcastMembershipError, DiagnosticEvent,
    HeartbeatNtpSampleObservation, HeartbeatTimestampSource, HeartbeatTimingObservation,
    InfoRequestEvent, JoinEvent, MembershipEvent, NetworkRuntime, NetworkRuntimeHandle,
    PeerLivenessEvent, RegistryRequestEvent, RequestInfoError, RequestRegistryError,
    RequestResourcesError, RequestSensorsError, SendJoinRequestError, SpawnError, UpdateError,
    UpdateReport,
};

#[cfg(feature = "app_instance")]
pub mod app_instance;

#[cfg(feature = "discovery_client")]
pub mod discovery_client;
