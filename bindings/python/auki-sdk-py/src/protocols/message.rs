//! Python client and full endpoint lifecycle for persistent Message v1 channels.

use std::sync::Arc;

use auki_protocols::message::{
    MessageChannelReceiver, MessageChannelRegistrationError, MessageChannelResource,
    MessageChannelSender, MessageClient, MessageEndpoint, MessageEvent, v1::ID,
};
use auki_registry::RegistryRef;
use parking_lot::Mutex;
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyAny, PyBytes, PyDict, PyModule},
};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::{
    PyAukiPeer,
    cleanup::{CleanupResult, DetachedCleanup, wait_cleanup},
};

use super::support::{
    enter_tokio_runtime, parse_peer_id, parse_python, parse_target, requester_to_python,
    runtime_error, to_python,
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum MessageChannelVariant {
    MessageChannel,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MessageChannelRecord {
    variant: MessageChannelVariant,
    owner_peer_id: String,
    resource_id: String,
    clock: RegistryRef,
}

impl From<&MessageChannelResource> for MessageChannelRecord {
    fn from(resource: &MessageChannelResource) -> Self {
        Self {
            variant: MessageChannelVariant::MessageChannel,
            owner_peer_id: resource.owner_peer_id.to_string(),
            resource_id: resource.resource_id.clone(),
            clock: resource.clock.clone(),
        }
    }
}

fn channel_from_python(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
) -> PyResult<MessageChannelResource> {
    let record: MessageChannelRecord = parse_python(py, value, "Message channel")?;
    let resource = MessageChannelResource {
        owner_peer_id: record.owner_peer_id.parse().map_err(|error| {
            PyValueError::new_err(format!("invalid Message channel owner Peer ID: {error}"))
        })?,
        resource_id: record.resource_id,
        clock: record.clock,
    };
    resource
        .validate()
        .map_err(|error| PyValueError::new_err(format!("invalid Message channel: {error}")))?;
    Ok(resource)
}

fn channel_to_python(py: Python<'_>, resource: &MessageChannelResource) -> PyResult<PyObject> {
    to_python(py, &MessageChannelRecord::from(resource))
}

fn catalog_to_python(py: Python<'_>, resources: &[MessageChannelResource]) -> PyResult<PyObject> {
    let records = resources
        .iter()
        .map(MessageChannelRecord::from)
        .collect::<Vec<_>>();
    to_python(py, &records)
}

fn event_to_python(py: Python<'_>, event: MessageEvent) -> PyResult<PyObject> {
    let value = PyDict::new_bound(py);
    value.set_item("channel", channel_to_python(py, &event.channel)?)?;
    value.set_item("sender", requester_to_python(py, &event.sender)?)?;
    value.set_item("type", event.message.r#type)?;
    value.set_item("timestamp_ns", event.message.timestamp_ns)?;
    value.set_item("payload", PyBytes::new_bound(py, &event.message.payload))?;
    Ok(value.unbind().into_any())
}

/// Outbound Message v1 client backed by the portable Rust protocol.
#[pyclass(name = "AukiMessageClient", frozen)]
#[derive(Clone)]
pub(crate) struct PyAukiMessageClient {
    inner: MessageClient,
}

impl PyAukiMessageClient {
    fn from_inner(inner: MessageClient) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyAukiMessageClient {
    #[new]
    fn new(peer: &PyAukiPeer) -> Self {
        Self::from_inner(MessageClient::new(peer.protocols()))
    }

    #[getter]
    fn protocol(&self) -> &'static str {
        ID
    }

    /// Open a persistent channel using routes configured on the peer.
    fn open<'py>(
        &self,
        py: Python<'py>,
        remote_peer_id: String,
        channel: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let remote_peer_id = parse_peer_id(&remote_peer_id)?;
        let channel = channel_from_python(py, channel)?;
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let sender = client
                .open(remote_peer_id, &channel)
                .await
                .map_err(|error| runtime_error("open Message channel", error))?;
            Python::with_gil(|py| Py::new(py, PyAukiMessageSender::new(sender)))
        })
    }

    /// Open a persistent channel through one exact advertised route.
    fn open_exact<'py>(
        &self,
        py: Python<'py>,
        remote_peer_id: String,
        route: String,
        channel: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (remote_peer_id, route) = parse_target(&remote_peer_id, &route)?;
        let channel = channel_from_python(py, channel)?;
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let sender = client
                .open_exact(remote_peer_id, route, &channel)
                .await
                .map_err(|error| runtime_error("open exact Message channel", error))?;
            Python::with_gil(|py| Py::new(py, PyAukiMessageSender::new(sender)))
        })
    }
}

struct MessageEndpointOwner {
    endpoint: Mutex<Option<MessageEndpoint>>,
    cleanup: DetachedCleanup,
}

impl MessageEndpointOwner {
    fn new(endpoint: MessageEndpoint) -> Self {
        Self {
            endpoint: Mutex::new(Some(endpoint)),
            cleanup: DetachedCleanup::default(),
        }
    }

    fn declare(
        &self,
        resource: MessageChannelResource,
        receiver_capacity: usize,
    ) -> PyResult<MessageChannelReceiver> {
        self.endpoint
            .lock()
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Message endpoint is closed"))?
            .declare(resource, receiver_capacity)
            .map_err(declaration_error)
    }

    fn catalog(&self) -> PyResult<Vec<MessageChannelResource>> {
        Ok(self
            .endpoint
            .lock()
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Message endpoint is closed"))?
            .catalog())
    }

    fn begin_close(&self) -> watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            let close = self.endpoint.lock().take().map(MessageEndpoint::close);
            async move {
                match close {
                    Some(close) => close.await.map_err(|error| error.to_string()),
                    None => Ok(()),
                }
            }
        })
    }
}

impl Drop for MessageEndpointOwner {
    fn drop(&mut self) {
        let Some(endpoint) = self.endpoint.get_mut().take() else {
            return;
        };
        pyo3_async_runtimes::tokio::get_runtime().spawn(async move {
            let _ = endpoint.close().await;
        });
    }
}

fn declaration_error(error: MessageChannelRegistrationError) -> PyErr {
    match error {
        MessageChannelRegistrationError::Stopped => {
            PyRuntimeError::new_err("Message endpoint is closed")
        }
        error => PyValueError::new_err(format!("cannot declare Message channel: {error}")),
    }
}

/// Mounted inbound Message v1 service and its declared receivers.
#[pyclass(name = "AukiMessageEndpoint")]
pub(crate) struct PyAukiMessageEndpoint {
    owner: MessageEndpointOwner,
    client: MessageClient,
}

#[pymethods]
impl PyAukiMessageEndpoint {
    /// Mount Message v1 on one running peer.
    #[staticmethod]
    fn mount(peer: &PyAukiPeer) -> PyResult<Self> {
        let endpoint = enter_tokio_runtime(|| MessageEndpoint::mount(peer.protocols()))
            .map_err(|error| runtime_error("mount Message endpoint", error))?;
        let client = endpoint.client();
        Ok(Self {
            owner: MessageEndpointOwner::new(endpoint),
            client,
        })
    }

    #[getter]
    fn protocol(&self) -> &'static str {
        ID
    }

    #[getter]
    fn client(&self) -> PyAukiMessageClient {
        PyAukiMessageClient::from_inner(self.client.clone())
    }

    /// Declare one receiver-owned channel and its bounded native queue.
    fn declare(
        &self,
        py: Python<'_>,
        channel: &Bound<'_, PyAny>,
        receiver_capacity: usize,
    ) -> PyResult<PyAukiMessageReceiver> {
        let channel = channel_from_python(py, channel)?;
        let receiver = self.owner.declare(channel, receiver_capacity)?;
        Ok(PyAukiMessageReceiver::new(receiver))
    }

    /// Snapshot every currently declared channel as canonical Catalog v3 rows.
    fn catalog(&self, py: Python<'_>) -> PyResult<PyObject> {
        catalog_to_python(py, &self.owner.catalog()?)
    }

    /// Stop declarations and await all admitted handlers behind one detached barrier.
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let cleanup = self.owner.begin_close();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            wait_cleanup(cleanup)
                .await
                .map_err(|error| runtime_error("close Message endpoint", error))
        })
    }
}

struct MessageSenderOwner {
    sender: Mutex<Option<MessageChannelSender>>,
    cleanup: DetachedCleanup,
}

impl MessageSenderOwner {
    fn new(sender: MessageChannelSender) -> Self {
        Self {
            sender: Mutex::new(Some(sender)),
            cleanup: DetachedCleanup::default(),
        }
    }

    fn sender(&self) -> PyResult<MessageChannelSender> {
        self.sender
            .lock()
            .as_ref()
            .cloned()
            .ok_or_else(|| PyRuntimeError::new_err("Message sender is closed"))
    }

    fn begin_close(&self) -> watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            let close = self.sender.lock().take().map(MessageChannelSender::close);
            async move {
                match close {
                    Some(close) => close.await.map_err(|error| error.to_string()),
                    None => Ok(()),
                }
            }
        })
    }
}

impl Drop for MessageSenderOwner {
    fn drop(&mut self) {
        let Some(sender) = self.sender.get_mut().take() else {
            return;
        };
        pyo3_async_runtimes::tokio::get_runtime().spawn(async move {
            let _ = sender.close().await;
        });
    }
}

/// Persistent outbound Message v1 channel.
#[pyclass(name = "AukiMessageSender")]
pub(crate) struct PyAukiMessageSender {
    owner: MessageSenderOwner,
    remote_peer: auki_sdk_rs::AuthenticatedPeer,
    channel: MessageChannelResource,
    relayed: bool,
}

impl PyAukiMessageSender {
    fn new(sender: MessageChannelSender) -> Self {
        Self {
            remote_peer: sender.remote_peer().clone(),
            channel: sender.resource().clone(),
            relayed: sender.is_relayed(),
            owner: MessageSenderOwner::new(sender),
        }
    }
}

#[pymethods]
impl PyAukiMessageSender {
    /// Mutually authenticated receiver metadata without credentials or proofs.
    #[getter]
    fn remote_peer(&self, py: Python<'_>) -> PyResult<PyObject> {
        requester_to_python(py, &self.remote_peer)
    }

    #[getter]
    fn channel(&self, py: Python<'_>) -> PyResult<PyObject> {
        channel_to_python(py, &self.channel)
    }

    #[getter]
    fn relayed(&self) -> bool {
        self.relayed
    }

    /// Send one opaque typed message and await its exact acknowledgement.
    fn send<'py>(
        &self,
        py: Python<'py>,
        message_type: String,
        timestamp_ns: i64,
        payload: &Bound<'_, PyBytes>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let sender = self.owner.sender()?;
        let payload = payload.as_bytes().to_vec();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            sender
                .send(message_type, timestamp_ns, payload)
                .await
                .map_err(|error| runtime_error("send Message", error))
        })
    }

    /// Close the shared channel behind one detached, replayable cleanup barrier.
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let cleanup = self.owner.begin_close();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            wait_cleanup(cleanup)
                .await
                .map_err(|error| runtime_error("close Message sender", error))
        })
    }
}

struct MessageReceiverSlot {
    receiver: Option<MessageChannelReceiver>,
    closed: bool,
    next_pending: bool,
}

struct MessageReceiverState {
    slot: Mutex<MessageReceiverSlot>,
    channel: MessageChannelResource,
    cancel: watch::Sender<bool>,
    completed: watch::Sender<bool>,
    cleanup: DetachedCleanup,
}

impl MessageReceiverState {
    fn new(receiver: MessageChannelReceiver) -> Self {
        let channel = receiver.resource().clone();
        let (cancel, _) = watch::channel(false);
        let (completed, _) = watch::channel(false);
        Self {
            slot: Mutex::new(MessageReceiverSlot {
                receiver: Some(receiver),
                closed: false,
                next_pending: false,
            }),
            channel,
            cancel,
            completed,
            cleanup: DetachedCleanup::default(),
        }
    }

    fn begin_next(self: &Arc<Self>) -> PyResult<Option<PendingMessageReceiver>> {
        let mut slot = self.slot.lock();
        if slot.closed {
            return Ok(None);
        }
        if slot.next_pending {
            return Err(PyRuntimeError::new_err(
                "Message receiver already has a pending next()",
            ));
        }
        let receiver = slot
            .receiver
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("Message receiver is unavailable"))?;
        slot.next_pending = true;
        Ok(Some(PendingMessageReceiver {
            state: Arc::clone(self),
            receiver: Some(receiver),
        }))
    }

    fn finish_next(&self, receiver: MessageChannelReceiver, ended: bool) {
        let mut receiver = Some(receiver);
        let complete = {
            let mut slot = self.slot.lock();
            slot.next_pending = false;
            if slot.closed || ended {
                slot.closed = true;
                true
            } else {
                slot.receiver = receiver.take();
                false
            }
        };
        drop(receiver);
        if complete {
            self.cancel.send_replace(true);
            self.completed.send_replace(true);
        }
    }

    fn begin_close(&self) -> watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            self.cancel.send_replace(true);
            let receiver = {
                let mut slot = self.slot.lock();
                slot.closed = true;
                if slot.next_pending {
                    None
                } else {
                    slot.receiver.take()
                }
            };
            if let Some(receiver) = receiver {
                drop(receiver);
                self.completed.send_replace(true);
            }
            let completion = self.completed.subscribe();
            async move { wait_receiver_completion(completion).await }
        })
    }
}

struct PendingMessageReceiver {
    state: Arc<MessageReceiverState>,
    receiver: Option<MessageChannelReceiver>,
}

impl PendingMessageReceiver {
    fn receiver(&mut self) -> &mut MessageChannelReceiver {
        self.receiver
            .as_mut()
            .expect("a pending Message receive owns the native receiver")
    }

    fn finish(mut self, ended: bool) {
        let receiver = self
            .receiver
            .take()
            .expect("a pending Message receive finishes only once");
        self.state.finish_next(receiver, ended);
    }
}

impl Drop for PendingMessageReceiver {
    fn drop(&mut self) {
        if let Some(receiver) = self.receiver.take() {
            self.state.finish_next(receiver, false);
        }
    }
}

async fn receive_next(mut pending: PendingMessageReceiver) -> Option<MessageEvent> {
    let mut cancellation = pending.state.cancel.subscribe();
    let cancelled = *cancellation.borrow();
    let event = if cancelled {
        None
    } else {
        tokio::select! {
            biased;
            _ = cancellation.changed() => None,
            event = pending.receiver().recv() => event,
        }
    };
    let ended = event.is_none();
    pending.finish(ended);
    event
}

async fn wait_receiver_completion(mut completion: watch::Receiver<bool>) -> Result<(), String> {
    loop {
        if *completion.borrow_and_update() {
            return Ok(());
        }
        if completion.changed().await.is_err() {
            return Err("Message receiver cleanup ended without a result".into());
        }
    }
}

/// One bounded receiver declaration.
#[pyclass(name = "AukiMessageReceiver")]
pub(crate) struct PyAukiMessageReceiver {
    state: Arc<MessageReceiverState>,
}

impl PyAukiMessageReceiver {
    fn new(receiver: MessageChannelReceiver) -> Self {
        Self {
            state: Arc::new(MessageReceiverState::new(receiver)),
        }
    }
}

#[pymethods]
impl PyAukiMessageReceiver {
    #[getter]
    fn channel(&self, py: Python<'_>) -> PyResult<PyObject> {
        channel_to_python(py, &self.state.channel)
    }

    /// Receive one message dict, or `None` after close/undeclaration.
    /// Only one `next()` may be pending at a time.
    fn next<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let Some(pending) = self.state.begin_next()? else {
            return pyo3_async_runtimes::tokio::future_into_py(py, async move {
                Ok(Python::with_gil(|py| py.None()))
            });
        };
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let event = receive_next(pending).await;
            Python::with_gil(|py| match event {
                Some(event) => event_to_python(py, event),
                None => Ok(py.None()),
            })
        })
    }

    /// Undeclare the channel and await native receiver cleanup behind a detached barrier.
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let cleanup = self.state.begin_close();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            wait_cleanup(cleanup)
                .await
                .map_err(|error| runtime_error("close Message receiver", error))
        })
    }
}

impl Drop for PyAukiMessageReceiver {
    fn drop(&mut self) {
        self.state.begin_close();
    }
}

pub(super) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyAukiMessageClient>()?;
    module.add_class::<PyAukiMessageEndpoint>()?;
    module.add_class::<PyAukiMessageSender>()?;
    module.add_class::<PyAukiMessageReceiver>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use auki_protocols::message::v1::Message;
    use auki_sdk_rs::Identity;

    use super::super::support::requester;
    use super::*;

    fn channel() -> MessageChannelResource {
        let owner_peer_id = Identity::generate().peer_id();
        MessageChannelResource {
            owner_peer_id,
            resource_id: "events".into(),
            clock: RegistryRef {
                peer_id: owner_peer_id.to_string(),
                id: "session/monotonic".into(),
                hash: "clock-hash".into(),
            },
        }
    }

    #[test]
    fn channel_records_round_trip_in_catalog_shape() {
        Python::with_gil(|py| {
            let expected = channel();
            let value = channel_to_python(py, &expected).unwrap();
            let value = value.bind(py);
            assert_eq!(
                value
                    .get_item("variant")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "message_channel"
            );
            assert_eq!(channel_from_python(py, value).unwrap(), expected);

            let invalid = PyDict::new_bound(py);
            invalid.set_item("variant", "MessageChannel").unwrap();
            invalid
                .set_item("owner_peer_id", expected.owner_peer_id.to_string())
                .unwrap();
            invalid.set_item("resource_id", "events").unwrap();
            invalid
                .set_item(
                    "clock",
                    channel_to_python(py, &expected)
                        .unwrap()
                        .bind(py)
                        .get_item("clock")
                        .unwrap(),
                )
                .unwrap();
            assert!(channel_from_python(py, invalid.as_any()).is_err());
        });
    }

    #[test]
    fn message_events_preserve_authenticated_sender_and_binary_payload() {
        Python::with_gil(|py| {
            let channel = channel();
            let sender = requester(Identity::generate().peer_id());
            let value = event_to_python(
                py,
                MessageEvent {
                    channel,
                    sender,
                    message: Message {
                        r#type: "example.event".into(),
                        timestamp_ns: i64::MAX,
                        payload: vec![0, 1, 127, 255],
                    },
                },
            )
            .unwrap();
            let value = value.bind(py);
            assert_eq!(
                value.get_item("type").unwrap().extract::<String>().unwrap(),
                "example.event"
            );
            assert_eq!(
                value
                    .get_item("timestamp_ns")
                    .unwrap()
                    .extract::<i64>()
                    .unwrap(),
                i64::MAX
            );
            let payload = value.get_item("payload").unwrap();
            assert_eq!(
                payload.downcast::<PyBytes>().unwrap().as_bytes(),
                &[0, 1, 127, 255]
            );
            let sender = value.get_item("sender").unwrap();
            assert_eq!(
                sender
                    .get_item("peer_type")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "native_app"
            );
        });
    }

    #[test]
    fn module_registers_the_complete_message_surface() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_sdk").unwrap();
            register(&module).unwrap();
            for name in [
                "AukiMessageClient",
                "AukiMessageEndpoint",
                "AukiMessageSender",
                "AukiMessageReceiver",
            ] {
                assert!(module.getattr(name).is_ok(), "missing {name}");
            }
        });
    }
}
