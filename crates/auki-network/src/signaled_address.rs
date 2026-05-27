use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use std::{error::Error, fmt};

pub const SIGNALED_ADDRESS_PREFIX: &str = "/auki-webrtc-signaling/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSignaledAddress {
    pub discovery_url: String,
    pub peer_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignaledAddressError {
    MissingDiscoveryUrl,
    MissingPeerId,
    InvalidAddress(String),
    InvalidDiscoveryEncoding,
}

impl fmt::Display for SignaledAddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDiscoveryUrl => f.write_str("missing discovery url"),
            Self::MissingPeerId => f.write_str("missing peer id"),
            Self::InvalidAddress(address) => write!(f, "invalid signaled address: {address}"),
            Self::InvalidDiscoveryEncoding => f.write_str("invalid discovery url encoding"),
        }
    }
}

impl Error for SignaledAddressError {}

pub fn format_signaled_address(
    discovery_url: impl AsRef<str>,
    peer_id: impl AsRef<str>,
) -> Result<String, SignaledAddressError> {
    let discovery_url = discovery_url.as_ref().trim_end_matches('/');
    let peer_id = peer_id.as_ref();
    if discovery_url.is_empty() {
        return Err(SignaledAddressError::MissingDiscoveryUrl);
    }
    if peer_id.is_empty() {
        return Err(SignaledAddressError::MissingPeerId);
    }
    Ok(format!(
        "{SIGNALED_ADDRESS_PREFIX}{}/p2p/{peer_id}",
        URL_SAFE_NO_PAD.encode(discovery_url.as_bytes())
    ))
}

pub fn parse_signaled_address(
    address: &str,
) -> Result<ParsedSignaledAddress, SignaledAddressError> {
    let Some(rest) = address.strip_prefix(SIGNALED_ADDRESS_PREFIX) else {
        return Err(SignaledAddressError::InvalidAddress(address.to_string()));
    };
    let Some((encoded_url, peer_id)) = rest.split_once("/p2p/") else {
        return Err(SignaledAddressError::InvalidAddress(address.to_string()));
    };
    if encoded_url.is_empty() || peer_id.is_empty() {
        return Err(SignaledAddressError::InvalidAddress(address.to_string()));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded_url)
        .map_err(|_| SignaledAddressError::InvalidDiscoveryEncoding)?;
    let discovery_url =
        String::from_utf8(bytes).map_err(|_| SignaledAddressError::InvalidDiscoveryEncoding)?;
    if discovery_url.is_empty() {
        return Err(SignaledAddressError::MissingDiscoveryUrl);
    }
    Ok(ParsedSignaledAddress {
        discovery_url,
        peer_id: peer_id.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_parses_discovery_signaling_address() {
        let peer_id = "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar";
        let address = format_signaled_address("http://127.0.0.1:8080/", peer_id).unwrap();

        assert_eq!(
            address,
            "/auki-webrtc-signaling/aHR0cDovLzEyNy4wLjAuMTo4MDgw/p2p/12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"
        );

        let parsed = parse_signaled_address(&address).unwrap();
        assert_eq!(parsed.discovery_url, "http://127.0.0.1:8080");
        assert_eq!(parsed.peer_id, peer_id);
    }

    #[test]
    fn rejects_malformed_signaled_addresses() {
        assert!(parse_signaled_address("/ip4/127.0.0.1/tcp/4001").is_err());
        assert!(parse_signaled_address("/auki-webrtc-signaling/not-base64/p2p/peer").is_err());
        assert!(format_signaled_address("", "peer").is_err());
        assert!(format_signaled_address("http://127.0.0.1:8080", "").is_err());
    }
}
