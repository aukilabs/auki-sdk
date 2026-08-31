use std::net::IpAddr;

use reqwest::Url;

/// DMS HTTP base used by [`AukiWebPeerConfig::dev`].
pub const DEV_DMS_BASE_URL: &str = "https://dms.dev.aukiverse.com/v1/";

/// Fixed, intentionally small configuration for one browser Peer.
///
/// Browser peers always request one public relay. Retry, polling, booking
/// duration, and renewal policy remain SDK-owned mechanics.
#[derive(Clone, Debug)]
pub struct AukiWebPeerConfig {
    dms_base: Url,
}

impl AukiWebPeerConfig {
    /// Configure one browser Peer against an exact DMS HTTP base.
    pub fn new(dms_base: impl AsRef<str>) -> Result<Self, AukiWebPeerConfigError> {
        let dms_base =
            Url::parse(dms_base.as_ref()).map_err(|_| AukiWebPeerConfigError::InvalidDmsBase)?;
        if dms_base.cannot_be_a_base()
            || !dms_base.username().is_empty()
            || dms_base.password().is_some()
            || dms_base.query().is_some()
            || dms_base.fragment().is_some()
            || dms_base.host_str().is_none()
            || !safe_scheme_and_host(&dms_base)
        {
            return Err(AukiWebPeerConfigError::InvalidDmsBase);
        }
        Ok(Self { dms_base })
    }

    /// Configure one browser Peer against the shared development DMS.
    pub fn dev() -> Self {
        Self::new(DEV_DMS_BASE_URL).expect("the built-in development DMS URL is valid")
    }

    /// Exact normalized DMS base URL.
    pub fn dms_base_url(&self) -> &str {
        self.dms_base.as_str()
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn dms_base(&self) -> &Url {
        &self.dms_base
    }
}

fn safe_scheme_and_host(url: &Url) -> bool {
    match url.scheme() {
        "https" => true,
        "http" => url
            .host_str()
            .and_then(|host| host.trim_matches(['[', ']']).parse::<IpAddr>().ok())
            .is_some_and(|address| address.is_loopback()),
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AukiWebPeerConfigError {
    #[error(
        "DMS base must be HTTPS (or literal loopback HTTP) without credentials, query, or fragment"
    )]
    InvalidDmsBase,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_https_and_literal_loopback_http_only() {
        for valid in [
            "https://dms.dev.aukiverse.com/v1/",
            "http://127.0.0.1:8080/v1/",
            "http://[::1]:8080/v1/",
        ] {
            assert!(AukiWebPeerConfig::new(valid).is_ok(), "rejected {valid}");
        }
        for invalid in [
            "http://dms.example.com/v1/",
            "https://user@dms.example.com/v1/",
            "https://dms.example.com/v1/?query=yes",
            "https://dms.example.com/v1/#fragment",
            "not-a-url",
        ] {
            assert!(
                AukiWebPeerConfig::new(invalid).is_err(),
                "accepted {invalid}"
            );
        }
    }
}
