use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::io::{AsyncRead, AsyncWrite};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

use crate::{
    ApplicationProtocol, ApplicationProtocolSpec, AuthenticatedApplicationStream,
    AuthenticatedStream, Error, Multiaddr, Node, PeerId, Protocol, RelayRouteHandle, Result,
    SessionRequirements,
};

/// One explicitly selected direct or circuit route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExactRoute {
    Direct(Multiaddr),
    Circuit(Multiaddr),
}

impl ExactRoute {
    pub fn from_multiaddr(route: Multiaddr) -> Self {
        if route
            .iter()
            .any(|protocol| protocol == Protocol::P2pCircuit)
        {
            Self::Circuit(route)
        } else {
            Self::Direct(route)
        }
    }

    pub fn multiaddr(&self) -> &Multiaddr {
        match self {
            Self::Direct(route) | Self::Circuit(route) => route,
        }
    }

    pub fn is_circuit(&self) -> bool {
        matches!(self, Self::Circuit(_))
    }
}

/// An authenticated application stream retaining ownership of its exact relay
/// circuit until explicit close or Drop.
pub struct AuthenticatedRouteStream {
    stream: Option<AuthenticatedStream>,
    relay: Option<RelayRouteGuard>,
}

impl AuthenticatedRouteStream {
    fn direct(stream: AuthenticatedStream) -> Self {
        Self {
            stream: Some(stream),
            relay: None,
        }
    }

    fn relayed(node: Node, stream: AuthenticatedStream, route: RelayRouteHandle) -> Self {
        Self {
            stream: Some(stream),
            relay: Some(RelayRouteGuard::new(node, route)),
        }
    }

    pub fn remote_peer(&self) -> &crate::AuthenticatedPeer {
        self.stream
            .as_ref()
            .expect("an authenticated route stream remains present until close")
            .remote_peer()
    }

    pub fn is_relayed(&self) -> bool {
        self.relay.is_some()
    }

    pub async fn close(mut self) -> Result<()> {
        drop(self.stream.take());
        if let Some(mut relay) = self.relay.take() {
            relay.close().await?;
        }
        Ok(())
    }
}

impl AsyncRead for AuthenticatedRouteStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(
            self.stream
                .as_mut()
                .expect("an authenticated route stream remains present until close"),
        )
        .poll_read(context, buffer)
    }
}

impl AsyncWrite for AuthenticatedRouteStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(
            self.stream
                .as_mut()
                .expect("an authenticated route stream remains present until close"),
        )
        .poll_write(context, buffer)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(
            self.stream
                .as_mut()
                .expect("an authenticated route stream remains present until close"),
        )
        .poll_flush(context)
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(
            self.stream
                .as_mut()
                .expect("an authenticated route stream remains present until close"),
        )
        .poll_close(context)
    }
}

struct RelayRouteGuard {
    node: Node,
    route: Option<RelayRouteHandle>,
}

impl RelayRouteGuard {
    fn new(node: Node, route: RelayRouteHandle) -> Self {
        Self {
            node,
            route: Some(route),
        }
    }

    async fn close(&mut self) -> Result<()> {
        let Some(route) = self.route.as_ref().cloned() else {
            return Ok(());
        };
        self.node.close_relay_route(&route).await?;
        self.route.take();
        Ok(())
    }

    fn route(&self) -> &RelayRouteHandle {
        self.route
            .as_ref()
            .expect("a relay route guard retains its handle until close")
    }

    fn take(&mut self) -> RelayRouteHandle {
        self.route
            .take()
            .expect("a relay route guard retains its handle until transfer")
    }
}

impl Drop for RelayRouteGuard {
    fn drop(&mut self) {
        let Some(route) = self.route.take() else {
            return;
        };
        let node = self.node.clone();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        runtime.spawn(async move {
            let _ = node.close_relay_route(&route).await;
        });
    }
}

impl Node {
    /// Open one authenticated application stream over exactly the supplied
    /// route. Circuit routes retain an RAII close guard and never fall back to
    /// a direct or sibling-relay connection.
    pub async fn open_exact_route(
        &self,
        remote_peer_id: PeerId,
        route: ExactRoute,
        protocol: ApplicationProtocol,
        requirements: SessionRequirements,
    ) -> Result<AuthenticatedRouteStream> {
        match route {
            ExactRoute::Direct(route) => self
                .open(remote_peer_id, vec![route], protocol, requirements)
                .await
                .map(AuthenticatedRouteStream::direct),
            ExactRoute::Circuit(route) => {
                let relay = self
                    .connect_relayed_reusing_admission(route, &requirements)
                    .await?;
                let mut guard = RelayRouteGuard::new(self.clone(), relay);
                match self
                    .open_relayed(guard.route(), protocol, requirements)
                    .await
                {
                    Ok(stream) => {
                        let relay = guard.take();
                        Ok(AuthenticatedRouteStream::relayed(
                            self.clone(),
                            stream,
                            relay,
                        ))
                    }
                    Err(open_error) => match guard.close().await {
                        Ok(()) => Err(open_error),
                        Err(close_error) => Err(close_error),
                    },
                }
            }
        }
    }
}

/// Supervised inbound protocol task owned by the shared P2P runtime.
pub struct ApplicationProtocolServer {
    stop: CancellationToken,
    task: Option<JoinHandle<()>>,
}

impl ApplicationProtocolServer {
    pub async fn shutdown(mut self) -> Result<()> {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            task.await.map_err(Error::ProtocolTask)?;
        }
        Ok(())
    }
}

impl Drop for ApplicationProtocolServer {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Node {
    /// Register and supervise one authenticated inbound application protocol.
    pub fn serve<H, F>(
        &self,
        spec: ApplicationProtocolSpec,
        requirements: SessionRequirements,
        shutdown: &CancellationToken,
        handler: H,
    ) -> Result<ApplicationProtocolServer>
    where
        H: Fn(AuthenticatedApplicationStream) -> F + Send + Sync + 'static,
        F: Future<Output = ()> + Send + 'static,
    {
        let mut incoming = self.accept(spec.protocol().clone(), requirements)?;
        let stop = shutdown.child_token();
        let task_stop = stop.clone();
        let handler = Arc::new(handler);
        let max_concurrency = spec.max_concurrency();
        let max_frame_bytes = spec.max_frame_bytes();
        let task = tokio::spawn(async move {
            let mut handlers = JoinSet::new();
            loop {
                tokio::select! {
                    biased;
                    _ = task_stop.cancelled() => break,
                    completed = handlers.join_next(), if !handlers.is_empty() => {
                        if let Some(Err(error)) = completed {
                            tracing::warn!(%error, "authenticated application protocol handler failed");
                        }
                    }
                    accepted = incoming.accept(), if handlers.len() < max_concurrency => {
                        let Some(accepted) = accepted else { break; };
                        let stream = match accepted {
                            Ok(stream) => stream,
                            Err(error) => {
                                tracing::warn!(%error, "authenticated application protocol session was rejected");
                                continue;
                            }
                        };
                        let handler = Arc::clone(&handler);
                        handlers.spawn(async move {
                            handler(AuthenticatedApplicationStream::new(stream, max_frame_bytes))
                                .await
                        });
                    }
                }
            }
            handlers.abort_all();
            while handlers.join_next().await.is_some() {}
        });
        Ok(ApplicationProtocolServer {
            stop,
            task: Some(task),
        })
    }
}
