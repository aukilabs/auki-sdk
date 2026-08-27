use std::fmt;

use libp2p::{identity, PeerId};

use crate::{Error, Result};

/// One process-lifetime Ed25519 identity shared by DDS proof and libp2p Noise.
#[derive(Clone)]
pub struct Identity {
    keypair: identity::Keypair,
    peer_id: PeerId,
}

/// Narrow, cloneable capability for proving ownership of a pre-join Peer ID.
///
/// The handle deliberately exposes only the public identity material DDS needs
/// and exact challenge signing. It cannot serialize or otherwise reveal the
/// private key.
#[derive(Clone)]
pub struct PeerIdentityProof {
    identity: Identity,
}

impl fmt::Debug for PeerIdentityProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PeerIdentityProof")
            .field("peer_id", &self.identity.peer_id)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for Identity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Identity")
            .field("peer_id", &self.peer_id)
            .field("private_key", &"[redacted]")
            .finish()
    }
}

impl Identity {
    /// Explicitly generate a new ephemeral identity.
    ///
    /// Production hosts that require a stable Peer ID must load canonical
    /// protobuf bytes or supply a stable seed and propagate any input error;
    /// they must not call this as a recovery path.
    pub fn generate() -> Self {
        Self::from_keypair(identity::Keypair::generate_ed25519())
    }

    /// Construct the canonical libp2p identity from a 32-byte Ed25519 seed.
    ///
    /// This wallet-agnostic constructor lets a host derive stable seed material
    /// elsewhere. A wallet-based host can derive `Wallet::derive_child("peer/v1")`
    /// and pass only the resulting 32 bytes here. The caller's buffer is not
    /// modified.
    pub fn from_ed25519_seed(seed: &[u8; 32]) -> Self {
        let mut seed_copy = *seed;
        let secret = identity::ed25519::SecretKey::try_from_bytes(&mut seed_copy)
            .expect("every 32-byte value is a valid Ed25519 seed");
        let keypair = identity::ed25519::Keypair::from(secret);
        Self::from_keypair(identity::Keypair::from(keypair))
    }

    /// Restore one canonical Ed25519 libp2p private key.
    ///
    /// The encoding is the cross-language protobuf format used by
    /// `libp2p-identity` and go-libp2p's `crypto.MarshalPrivateKey`. Unknown
    /// fields, non-canonical encodings, and other key algorithms are rejected.
    pub fn from_protobuf_encoding(bytes: &[u8]) -> Result<Self> {
        let keypair = identity::Keypair::from_protobuf_encoding(bytes)
            .map_err(|_| Error::InvalidIdentityPrivateKey)?;
        if keypair.key_type() != identity::KeyType::Ed25519 {
            return Err(Error::UnsupportedIdentityKeyType);
        }
        let canonical = keypair
            .to_protobuf_encoding()
            .map_err(|_| Error::InvalidIdentityPrivateKey)?;
        if canonical != bytes {
            return Err(Error::InvalidIdentityPrivateKey);
        }
        Ok(Self::from_keypair(keypair))
    }

    /// Export the canonical private-key encoding accepted by
    /// [`Identity::from_protobuf_encoding`]. Treat the returned bytes as a
    /// secret.
    pub fn to_protobuf_encoding(&self) -> Result<Vec<u8>> {
        self.keypair
            .to_protobuf_encoding()
            .map_err(|_| Error::InvalidIdentityPrivateKey)
    }

    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    pub fn public_key_protobuf(&self) -> Vec<u8> {
        self.keypair.public().encode_protobuf()
    }

    /// libp2p public key bound to [`Self::peer_id`]. Safe to publish.
    pub fn public_key(&self) -> identity::PublicKey {
        self.keypair.public()
    }

    /// Create the pre-join proof capability used by an external authority.
    pub fn proof(&self) -> PeerIdentityProof {
        PeerIdentityProof {
            identity: self.clone(),
        }
    }

    /// Signs the exact challenge bytes supplied by DDS without hashing,
    /// prefixing, or otherwise transforming them first.
    pub(crate) fn sign_challenge(&self, challenge: &[u8]) -> Result<Vec<u8>> {
        self.keypair
            .sign(challenge)
            .map_err(|error| Error::IdentitySigning(error.to_string()))
    }

    pub(crate) fn keypair(&self) -> identity::Keypair {
        self.keypair.clone()
    }

    fn from_keypair(keypair: identity::Keypair) -> Self {
        let peer_id = keypair.public().to_peer_id();
        Self { keypair, peer_id }
    }
}

impl PeerIdentityProof {
    pub fn peer_id(&self) -> PeerId {
        self.identity.peer_id()
    }

    pub fn public_key_protobuf(&self) -> Vec<u8> {
        self.identity.public_key_protobuf()
    }

    /// Sign the exact challenge bytes supplied by DDS.
    pub fn sign_challenge(&self, challenge: &[u8]) -> Result<Vec<u8>> {
        self.identity.sign_challenge(challenge)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protobuf_round_trip_preserves_peer_id_and_signing_identity() {
        let original = Identity::from_ed25519_seed(&[9u8; 32]);
        let encoded = original.to_protobuf_encoding().unwrap();
        let restored = Identity::from_protobuf_encoding(&encoded).unwrap();

        assert_eq!(restored.peer_id(), original.peer_id());
        let challenge = b"persisted-p2p-identity";
        let signature = restored.sign_challenge(challenge).unwrap();
        assert!(original.keypair().public().verify(challenge, &signature));
    }

    #[test]
    fn seed_constructor_is_deterministic_without_mutating_the_caller() {
        let seed = [11u8; 32];
        let snapshot = seed;
        let first = Identity::from_ed25519_seed(&seed);
        let second = Identity::from_ed25519_seed(&seed);

        assert_eq!(seed, snapshot);
        assert_eq!(first.peer_id(), second.peer_id());
        assert_eq!(
            first.to_protobuf_encoding().unwrap(),
            second.to_protobuf_encoding().unwrap()
        );
    }

    #[test]
    fn malformed_and_noncanonical_private_keys_fail_closed() {
        assert!(matches!(
            Identity::from_protobuf_encoding(b"not-a-private-key"),
            Err(Error::InvalidIdentityPrivateKey)
        ));

        let identity = Identity::generate();
        let mut encoded = identity.to_protobuf_encoding().unwrap();
        encoded.extend_from_slice(&[0x18, 0x00]);
        assert!(matches!(
            Identity::from_protobuf_encoding(&encoded),
            Err(Error::InvalidIdentityPrivateKey)
        ));
    }

    #[test]
    fn non_ed25519_private_key_encoding_fails_closed() {
        // Canonical protobuf field layout for a libp2p private key with key
        // type 2 (Secp256k1) and a 32-byte payload. The crate deliberately
        // enables only Ed25519, so decoding must fail without accepting or
        // substituting another identity.
        let mut encoded = vec![0x08, 0x02, 0x12, 0x20];
        encoded.extend_from_slice(&[1u8; 32]);
        assert!(Identity::from_protobuf_encoding(&encoded).is_err());
    }

    #[test]
    fn debug_output_redacts_private_key_material() {
        let identity = Identity::from_ed25519_seed(&[0xabu8; 32]);
        let debug = format!("{identity:?}");
        assert!(debug.contains(&identity.peer_id().to_string()));
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains("171, 171"));
    }

    #[test]
    fn proof_exposes_only_public_identity_and_exact_signing() {
        let identity = Identity::from_ed25519_seed(&[0x42; 32]);
        let proof = identity.proof();

        assert_eq!(proof.peer_id(), identity.peer_id());
        assert_eq!(proof.public_key_protobuf(), identity.public_key_protobuf());
        let challenge = b"exact-dds-challenge";
        let signature = proof.sign_challenge(challenge).unwrap();
        assert!(identity.public_key().verify(challenge, &signature));
        assert!(!identity.public_key().verify(b"other", &signature));
        assert!(!format!("{proof:?}").contains("private"));
    }
}
