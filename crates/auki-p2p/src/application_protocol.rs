//! Bounded application protocol identifiers shared by native and browser peers.

use libp2p::StreamProtocol;

use crate::{Error, Result};

const APPLICATION_PROTOCOL_MAX_BYTES: usize = 255;
const APPLICATION_COMPONENT_MAX_BYTES: usize = 64;
const VERSION_COMPONENT_MAX_BYTES: usize = 32;

/// A bounded, explicitly versioned libp2p application protocol ID.
///
/// IDs use `/<name>[/<name>...]/<version>`. Name components are bounded
/// lowercase ASCII identifiers; versions are bounded numeric components with
/// an optional `v` prefix.
///
/// Product owners choose their own namespace. The top-level `/auki/`
/// namespace is reserved for the authenticated SDK protocol family.
#[derive(Clone, Debug)]
pub struct ApplicationProtocol(StreamProtocol);

impl ApplicationProtocol {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.len() > APPLICATION_PROTOCOL_MAX_BYTES {
            return Err(Error::InvalidProtocol(format!(
                "protocol ID exceeds {APPLICATION_PROTOCOL_MAX_BYTES} bytes"
            )));
        }
        let components: Vec<_> = value.split('/').collect();
        let sdk_application = components.len() == 6
            && components[0].is_empty()
            && components[1] == "auki"
            && components[2] == "auth"
            && components[3] == "1"
            && is_application_component(components[4])
            && is_version_component(components[5]);
        let product_application = components.len() >= 3
            && components[0].is_empty()
            && components[1] != "auki"
            && components[1..components.len() - 1]
                .iter()
                .all(|component| is_application_component(component))
            && is_version_component(components.last().copied().unwrap_or_default());
        if !sdk_application && !product_application {
            return Err(Error::InvalidProtocol(
                "expected a bounded /<namespace>/.../<version> ID; /auki/ is reserved for /auki/auth/1/<application>/<version>"
                    .into(),
            ));
        }
        let protocol = StreamProtocol::try_from_owned(value)
            .map_err(|error| Error::InvalidProtocol(error.to_string()))?;
        Ok(Self(protocol))
    }

    pub(crate) fn stream_protocol(&self) -> StreamProtocol {
        self.0.clone()
    }
}

fn is_application_component(component: &str) -> bool {
    if component.is_empty() || component.len() > APPLICATION_COMPONENT_MAX_BYTES {
        return false;
    }
    let mut bytes = component.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

fn is_version_component(component: &str) -> bool {
    if component.is_empty() || component.len() > VERSION_COMPONENT_MAX_BYTES {
        return false;
    }
    let bytes = component.strip_prefix('v').unwrap_or(component).as_bytes();
    bytes.first().is_some_and(u8::is_ascii_digit)
        && bytes.last().is_some_and(u8::is_ascii_digit)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'-'))
        && !bytes
            .windows(2)
            .any(|pair| !pair[0].is_ascii_digit() && !pair[1].is_ascii_digit())
}
