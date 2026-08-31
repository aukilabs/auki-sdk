//! Bounded application protocol contracts shared by native and browser peers.

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::io::{AsyncRead, AsyncWrite};
use libp2p::StreamProtocol;

use crate::{AuthenticatedStream, Error, Result};

const APPLICATION_PROTOCOL_MAX_BYTES: usize = 255;
const APPLICATION_COMPONENT_MAX_BYTES: usize = 64;
const VERSION_COMPONENT_MAX_BYTES: usize = 32;
pub const APPLICATION_PROTOCOL_MAX_CONCURRENCY: usize = 1_024;
pub const APPLICATION_PROTOCOL_MAX_FRAME_BYTES: u32 = 64 * 1024 * 1024;

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

/// One explicit application protocol and its host-side resource bounds.
///
/// Mutual-authentication requirements remain runtime policy. The same spec can
/// therefore be mounted by a native or browser peer without embedding one
/// platform's executor or transport choices.
#[derive(Clone, Debug)]
pub struct ApplicationProtocolSpec {
    protocol: ApplicationProtocol,
    max_concurrency: usize,
    max_frame_bytes: u32,
}

impl ApplicationProtocolSpec {
    pub fn new(
        protocol: ApplicationProtocol,
        max_concurrency: usize,
        max_frame_bytes: u32,
    ) -> Result<Self> {
        if !(1..=APPLICATION_PROTOCOL_MAX_CONCURRENCY).contains(&max_concurrency) {
            return Err(Error::InvalidProtocol(format!(
                "protocol concurrency must be between 1 and {APPLICATION_PROTOCOL_MAX_CONCURRENCY}"
            )));
        }
        if !(1..=APPLICATION_PROTOCOL_MAX_FRAME_BYTES).contains(&max_frame_bytes) {
            return Err(Error::InvalidProtocol(format!(
                "protocol frame bound must be between 1 and {APPLICATION_PROTOCOL_MAX_FRAME_BYTES} bytes"
            )));
        }
        Ok(Self {
            protocol,
            max_concurrency,
            max_frame_bytes,
        })
    }

    pub fn protocol(&self) -> &ApplicationProtocol {
        &self.protocol
    }

    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    pub fn max_frame_bytes(&self) -> u32 {
        self.max_frame_bytes
    }
}

/// Mutually authenticated application stream plus its declared frame bound.
///
/// The transport cannot infer an application's framing. Protocol codecs must
/// reject frames larger than [`Self::max_frame_bytes`].
pub struct AuthenticatedApplicationStream {
    stream: AuthenticatedStream,
    max_frame_bytes: u32,
}

impl AuthenticatedApplicationStream {
    pub(crate) fn new(stream: AuthenticatedStream, max_frame_bytes: u32) -> Self {
        Self {
            stream,
            max_frame_bytes,
        }
    }

    pub fn remote_peer(&self) -> &crate::AuthenticatedPeer {
        self.stream.remote_peer()
    }

    pub fn max_frame_bytes(&self) -> u32 {
        self.max_frame_bytes
    }
}

impl AsyncRead for AuthenticatedApplicationStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for AuthenticatedApplicationStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(context, buffer)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(context)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_close(context)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn protocol() -> ApplicationProtocol {
        ApplicationProtocol::new("/example/spec/1.0.0").unwrap()
    }

    #[test]
    fn application_spec_enforces_exact_resource_bounds() {
        assert!(ApplicationProtocolSpec::new(protocol(), 0, 1).is_err());
        assert!(ApplicationProtocolSpec::new(
            protocol(),
            APPLICATION_PROTOCOL_MAX_CONCURRENCY + 1,
            1,
        )
        .is_err());
        assert!(ApplicationProtocolSpec::new(protocol(), 1, 0).is_err());
        assert!(ApplicationProtocolSpec::new(
            protocol(),
            1,
            APPLICATION_PROTOCOL_MAX_FRAME_BYTES + 1,
        )
        .is_err());

        let exact = ApplicationProtocolSpec::new(
            protocol(),
            APPLICATION_PROTOCOL_MAX_CONCURRENCY,
            APPLICATION_PROTOCOL_MAX_FRAME_BYTES,
        )
        .unwrap();
        assert_eq!(
            exact.max_concurrency(),
            APPLICATION_PROTOCOL_MAX_CONCURRENCY
        );
        assert_eq!(
            exact.max_frame_bytes(),
            APPLICATION_PROTOCOL_MAX_FRAME_BYTES
        );
    }
}
