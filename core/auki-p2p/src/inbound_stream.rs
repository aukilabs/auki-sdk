//! Bounded native inbound handoff around the stock stream behaviour.
//!
//! libp2p-stream uses a zero-buffer channel with try_send for inbound streams.
//! A burst can overflow it before the receiving task is scheduled. Intercept
//! negotiated inbound streams at the handler and give each registered protocol
//! a finite queue. Outbound routing and protocol negotiation remain delegated.

use std::{
    collections::HashMap,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use async_channel::{Receiver, Sender};
use futures::Stream as FuturesStream;
use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        handler::ConnectionEvent, ConnectionDenied, ConnectionHandler, ConnectionHandlerEvent,
        ConnectionId, FromSwarm, NetworkBehaviour, SubstreamProtocol, THandler, THandlerInEvent,
        THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId, Stream, StreamProtocol,
};
use parking_lot::Mutex;

const INBOUND_QUEUE_CAPACITY: usize = 64;
type Queues = Arc<Mutex<HashMap<StreamProtocol, Sender<(PeerId, Stream)>>>>;
type Delegate = <libp2p_stream::Behaviour as NetworkBehaviour>::ConnectionHandler;

pub struct Behaviour {
    inner: libp2p_stream::Behaviour,
    queues: Queues,
}

impl Behaviour {
    pub fn new() -> Self {
        Self {
            inner: libp2p_stream::Behaviour::new(),
            queues: Arc::default(),
        }
    }

    pub fn new_control(&self) -> Control {
        Control {
            inner: self.inner.new_control(),
            queues: self.queues.clone(),
        }
    }
}

#[derive(Clone)]
pub struct Control {
    inner: libp2p_stream::Control,
    queues: Queues,
}

impl Control {
    pub fn accept(
        &mut self,
        protocol: StreamProtocol,
    ) -> Result<IncomingStreams, libp2p_stream::AlreadyRegistered> {
        let mut queues = self.queues.lock();
        queues.retain(|_, sender| !sender.is_closed());
        // Retain the stock registration for negotiation and duplicate checks.
        let registration = self.inner.accept(protocol.clone())?;
        let (sender, receiver) = async_channel::bounded(INBOUND_QUEUE_CAPACITY);
        queues.insert(protocol, sender);
        Ok(IncomingStreams {
            receiver: Box::pin(receiver),
            _registration: registration,
        })
    }
}

pub struct IncomingStreams {
    receiver: Pin<Box<Receiver<(PeerId, Stream)>>>,
    _registration: libp2p_stream::IncomingStreams,
}

impl Drop for IncomingStreams {
    fn drop(&mut self) {
        // The registry can retain its sender until the next registration or
        // inbound event. Close and drain now so queued streams do not outlive
        // the owning protocol or its awaited shutdown.
        self.receiver.close();
        while self.receiver.try_recv().is_ok() {}
    }
}

impl FuturesStream for IncomingStreams {
    type Item = (PeerId, Stream);
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.as_mut().poll_next(cx)
    }
}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = Handler;
    type ToSwarm = <libp2p_stream::Behaviour as NetworkBehaviour>::ToSwarm;

    fn handle_established_inbound_connection(
        &mut self,
        id: ConnectionId,
        peer: PeerId,
        local: &Multiaddr,
        remote: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(Handler {
            inner: self
                .inner
                .handle_established_inbound_connection(id, peer, local, remote)?,
            peer,
            queues: self.queues.clone(),
        })
    }

    fn handle_established_outbound_connection(
        &mut self,
        id: ConnectionId,
        peer: PeerId,
        address: &Multiaddr,
        role: Endpoint,
        port: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(Handler {
            inner: self
                .inner
                .handle_established_outbound_connection(id, peer, address, role, port)?,
            peer,
            queues: self.queues.clone(),
        })
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        self.inner.on_swarm_event(event);
    }

    fn on_connection_handler_event(
        &mut self,
        peer: PeerId,
        id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        self.inner.on_connection_handler_event(peer, id, event);
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        self.inner.poll(cx)
    }
}

pub struct Handler {
    inner: Delegate,
    peer: PeerId,
    queues: Queues,
}

impl ConnectionHandler for Handler {
    type FromBehaviour = <Delegate as ConnectionHandler>::FromBehaviour;
    type ToBehaviour = <Delegate as ConnectionHandler>::ToBehaviour;
    type InboundProtocol = <Delegate as ConnectionHandler>::InboundProtocol;
    type OutboundProtocol = <Delegate as ConnectionHandler>::OutboundProtocol;
    type InboundOpenInfo = <Delegate as ConnectionHandler>::InboundOpenInfo;
    type OutboundOpenInfo = <Delegate as ConnectionHandler>::OutboundOpenInfo;

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
        self.inner.listen_protocol()
    }

    fn connection_keep_alive(&self) -> bool {
        self.inner.connection_keep_alive()
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<
        ConnectionHandlerEvent<Self::OutboundProtocol, Self::OutboundOpenInfo, Self::ToBehaviour>,
    > {
        self.inner.poll(cx)
    }

    fn poll_close(&mut self, cx: &mut Context<'_>) -> Poll<Option<Self::ToBehaviour>> {
        self.inner.poll_close(cx)
    }

    fn on_behaviour_event(&mut self, event: Self::FromBehaviour) {
        self.inner.on_behaviour_event(event);
    }

    fn on_connection_event(
        &mut self,
        event: ConnectionEvent<
            Self::InboundProtocol,
            Self::OutboundProtocol,
            Self::InboundOpenInfo,
            Self::OutboundOpenInfo,
        >,
    ) {
        match event {
            ConnectionEvent::FullyNegotiatedInbound(event) => {
                let (stream, protocol) = event.protocol;
                let mut queues = self.queues.lock();
                if let Some(sender) = queues.get(&protocol) {
                    match sender.try_send((self.peer, stream)) {
                        Ok(()) => {}
                        Err(async_channel::TrySendError::Full(_)) => {
                            tracing::debug!(%protocol, "bounded inbound stream queue is full");
                        }
                        Err(async_channel::TrySendError::Closed(_)) => {
                            queues.remove(&protocol);
                        }
                    }
                }
            }
            other => self.inner.on_connection_event(other),
        }
    }
}
