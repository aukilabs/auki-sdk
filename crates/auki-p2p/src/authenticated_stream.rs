//! Authenticated application stream shared by native and browser runtimes.

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::io::{AsyncRead, AsyncWrite};
use libp2p::Stream;

use crate::AuthenticatedPeer;

/// The public byte-stream boundary. The inner libp2p stream is deliberately not
/// exposed and this wrapper can only be constructed after mutual DDS auth.
pub struct AuthenticatedStream {
    inner: Stream,
    remote: AuthenticatedPeer,
}

impl AuthenticatedStream {
    pub(crate) fn new(inner: Stream, remote: AuthenticatedPeer) -> Self {
        Self { inner, remote }
    }

    pub fn remote_peer(&self) -> &AuthenticatedPeer {
        &self.remote
    }
}

impl std::fmt::Debug for AuthenticatedStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthenticatedStream")
            .field("remote", &self.remote)
            .finish_non_exhaustive()
    }
}

impl AsyncRead for AuthenticatedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_read(context, buffer)
    }
}

impl AsyncWrite for AuthenticatedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(context, buffer)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(context)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_close(context)
    }
}
