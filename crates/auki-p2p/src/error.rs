use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid Ed25519 libp2p private key")]
    InvalidIdentityPrivateKey,
    #[error("libp2p identity private key must use Ed25519")]
    UnsupportedIdentityKeyType,
    #[error("identity path must contain a regular file")]
    IdentityFileNotRegular,
    #[error("identity path must not be a symbolic link")]
    IdentityFileSymlink,
    #[error("identity file is {actual} bytes, exceeding the {maximum}-byte safety limit")]
    IdentityFileTooLarge { actual: u64, maximum: u64 },
    #[error("identity file permissions are {mode:o}; expected exactly 0o600")]
    InsecureIdentityFilePermissions { mode: u32 },
    #[error("failed to sign libp2p identity proof: {0}")]
    IdentitySigning(String),
    #[error("invalid DDS verification key: {0}")]
    InvalidVerificationKey(#[from] jsonwebtoken::errors::Error),
    #[error("invalid DDS verification-key set: {0}")]
    InvalidVerificationKeySet(String),
    #[error("DDS verification-key generation {proposed} is older than live generation {current}")]
    StaleVerificationKeyGeneration { current: u64, proposed: u64 },
    #[error("DDS verification-key generation {0} conflicts with the installed key material")]
    VerificationKeyGenerationConflict(u64),
    #[error("DDS verification-key rotation must retain the former current key as previous")]
    VerificationKeyRotationMissingPrevious,
    #[error("DDS previous verification key cannot be retired before its overlap expires")]
    VerificationKeyOverlapActive,
    #[error("DDS verification keys are stale; the host must refresh them")]
    VerificationKeysStale,
    #[error("invalid DDS P2P token: {0}")]
    InvalidToken(String),
    #[error("DDS P2P token signature or registered claims are invalid: {0}")]
    TokenVerification(jsonwebtoken::errors::Error),
    #[error("no current DDS P2P token is installed")]
    MissingToken,
    #[error(
        "DDS P2P credential issued at {proposed_issued_at} is older than the current credential issued at {current_issued_at}"
    )]
    StaleCredential {
        current_issued_at: u64,
        proposed_issued_at: u64,
    },
    #[error("DDS P2P credentials issued at {0} carry conflicting signed claims")]
    CredentialIssuedAtConflict(u64),
    #[error(
        "DDS P2P credential expiration {credential_expiration} does not match host response expiration {expected_expiration}"
    )]
    CredentialExpirationMismatch {
        credential_expiration: u64,
        expected_expiration: u64,
    },
    #[error("authority update targets Domain {actual}, expected {expected}")]
    AuthorityDomainMismatch { expected: String, actual: String },
    #[error("authority update targets Peer {actual}, expected {expected}")]
    AuthorityPeerMismatch { expected: String, actual: String },
    #[error("authority credential expiration must be a future whole UTC second")]
    InvalidAuthorityExpiration,
    #[error("token Peer ID {token_peer_id} does not match Noise Peer ID {noise_peer_id}")]
    PeerIdMismatch {
        token_peer_id: String,
        noise_peer_id: String,
    },
    #[error("remote token is not authorized for required Domain {0}")]
    RemoteDomainMismatch(String),
    #[error("local token is not authorized for required Domain {0}")]
    LocalDomainMismatch(String),
    #[error("expected remote Peer ID {expected}, connected to {actual}")]
    UnexpectedRemotePeer { expected: String, actual: String },
    #[error("remote peer rejected mutual authentication")]
    RemoteRejected,
    #[error("mutual authentication timed out")]
    AuthenticationTimeout,
    #[error("authentication token frame exceeds the {0}-byte limit")]
    TokenFrameTooLarge(usize),
    #[error("authentication token is not valid UTF-8")]
    InvalidTokenEncoding,
    #[error("invalid application protocol: {0}")]
    InvalidProtocol(String),
    #[error("application protocol is already registered")]
    ProtocolAlreadyRegistered,
    #[error("at least one explicit remote TCP multiaddr is required")]
    MissingRemoteAddress,
    #[error("invalid remote multiaddr {address}: {reason}")]
    InvalidRemoteAddress { address: String, reason: String },
    #[error("failed to build libp2p transport: {0}")]
    TransportBuild(String),
    #[error("auki-p2p requires an active Tokio runtime")]
    RuntimeUnavailable,
    #[cfg(not(target_arch = "wasm32"))]
    #[error("authenticated application protocol task failed")]
    ProtocolTask(#[source] tokio::task::JoinError),
    #[error("failed to listen on {address}: {reason}")]
    Listen { address: String, reason: String },
    #[error("failed to open libp2p stream: {0}")]
    OpenStream(String),
    #[error(transparent)]
    TargetedStream(#[from] crate::targeted_stream::TargetedStreamError),
    #[error("failed to dial libp2p peer: {0}")]
    Dial(String),
    #[error("failed to resolve relay DNS name: {0}")]
    Dns(String),
    #[error("libp2p swarm task has stopped")]
    SwarmStopped,
    #[error("failed to disconnect libp2p peer {0}")]
    Disconnect(String),
    #[error("relay admission frame is empty or exceeds the {maximum}-byte limit")]
    RelayAdmissionFrameTooLarge { maximum: usize },
    #[error("relay admission response is malformed")]
    RelayAdmissionMalformed,
    #[error("relay admission was denied")]
    RelayAdmissionDenied,
    #[error("relay admission authority is already expired")]
    RelayAdmissionExpired,
    #[error("relay admission timed out")]
    RelayAdmissionTimeout,
    #[error(transparent)]
    RelayReservation(#[from] crate::relay::RelayReservationError),
    #[error("relay reservation confirmation was rejected: {0}")]
    RelayConfirmationRejected(crate::relay::RelayConfirmationRejection),
    #[error("relay reservation listener closed before confirmation: {0}")]
    RelayReservationClosed(String),
    #[error(
        "the first direct connection to relay {relay_peer_id} does not match the selected base {expected}; observed {actual}"
    )]
    RelayDirectConnectionMismatch {
        relay_peer_id: String,
        expected: String,
        actual: String,
    },
    #[error("invalid relay route {address}: {reason}")]
    InvalidRelayRoute { address: String, reason: String },
    #[error("relayed sessions require an explicit expected target Peer ID")]
    MissingExpectedRelayTarget,
    #[error("relay route handle belongs to a different node instance")]
    ForeignRelayRoute,
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
