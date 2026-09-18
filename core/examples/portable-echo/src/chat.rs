//! Optional Core Explorer example addon, not a stable SDK protocol.
use async_channel::{Receiver, Sender};
use auki_sdk::{AukiPeerProtocols, AukiProtocolRegistration, AukiProtocolSpec, Multiaddr, PeerId};
use futures::{
    AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, FutureExt, channel::oneshot, pin_mut,
};
use futures_timer::Delay;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

pub const PROTOCOL_ID: &str = "/example/core-explorer-chat/1.0.0";
pub const MAX_FRAME_BYTES: usize = 16384;
pub const CAPACITY: usize = 128;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const PENDING_TTL: Duration = Duration::from_secs(300);
const SESSION_TTL: Duration = Duration::from_secs(3600);
pub type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Frame {
    Pending {
        session_id: Uuid,
    },
    Paired {
        session_id: Uuid,
    },
    Message {
        session_id: Uuid,
        id: Uuid,
        text: String,
    },
    Ack {
        session_id: Uuid,
        id: Uuid,
    },
}

#[derive(Debug, Serialize)]
pub struct Event {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}
impl Event {
    pub fn json(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }
}
fn text_valid(text: &str) -> Result<()> {
    if text.is_empty() || text.len() > 2048 {
        Err("chat text must contain 1–2048 UTF-8 bytes".into())
    } else {
        Ok(())
    }
}
async fn timed<T>(duration: Duration, work: impl Future<Output = Result<T>>) -> Result<T> {
    let work = work.fuse();
    let timer = Delay::new(duration).fuse();
    pin_mut!(work, timer);
    futures::select_biased! { result = work => result, _ = timer => Err("chat operation timed out".into()) }
}
async fn read_frame<R: AsyncRead + Unpin>(read: &mut R) -> Result<Frame> {
    let mut header = [0; 4];
    read.read_exact(&mut header)
        .await
        .map_err(|_| "chat stream closed")?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME_BYTES - 4 {
        return Err("invalid chat frame length".into());
    }
    let mut body = vec![0; length];
    timed(IO_TIMEOUT, async {
        read.read_exact(&mut body)
            .await
            .map_err(|_| "chat body read failed".to_string())
    })
    .await?;
    serde_json::from_slice(&body).map_err(|_| "invalid chat frame".into())
}
async fn write_frame<W: AsyncWrite + Unpin>(write: &mut W, frame: &Frame) -> Result<()> {
    let body = serde_json::to_vec(frame).map_err(|_| "chat encode failed")?;
    if body.len() > MAX_FRAME_BYTES - 4 {
        return Err("chat frame too large".into());
    }
    timed(IO_TIMEOUT, async {
        write
            .write_all(&(body.len() as u32).to_be_bytes())
            .await
            .map_err(|_| "chat write failed")?;
        write
            .write_all(&body)
            .await
            .map_err(|_| "chat write failed")?;
        write.flush().await.map_err(|_| "chat flush failed".into())
    })
    .await
}

struct Command {
    id: Uuid,
    text: String,
    completed: oneshot::Sender<Result<()>>,
}
struct State {
    session: Uuid,
    peer: String,
    host: bool,
    paired: AtomicBool,
    approving: AtomicBool,
    closed: AtomicBool,
    commands: Sender<Command>,
    cancel: Sender<()>,
    done: Receiver<()>,
    cleanup: Mutex<Option<Result<()>>>,
    events: Sender<Event>,
}
impl State {
    fn event(&self, kind: &'static str, id: Option<Uuid>, text: Option<String>) -> Result<()> {
        self.events
            .try_send(Event {
                kind,
                session_id: self.host.then(|| self.session.to_string()),
                peer_id: self.host.then(|| self.peer.clone()),
                id: id.map(|id| id.to_string()),
                text,
            })
            .map_err(|_| {
                self.events.close();
                "chat event queue full or closed".into()
            })
    }
    fn stop(&self) {
        self.closed.store(true, Ordering::Release);
        self.commands.close();
        self.cancel.close();
    }
}
#[derive(Clone)]
pub struct Connection {
    state: Arc<State>,
    events: Receiver<Event>,
}
impl Connection {
    pub fn session_id(&self) -> String {
        self.state.session.to_string()
    }
    pub async fn next_event(&self) -> Result<Event> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err("chat closed".into());
        }
        self.events
            .recv()
            .await
            .map_err(|_| "chat closed or event consumer overloaded".into())
    }
    pub async fn send(&self, id: &str, text: String) -> Result<()> {
        text_valid(&text)?;
        let id = Uuid::parse_str(id).map_err(|_| "invalid message UUID")?;
        if self.state.closed.load(Ordering::Acquire) || !self.state.paired.load(Ordering::Acquire) {
            return Err("chat is not paired and live".into());
        }
        let (completed, result) = oneshot::channel();
        self.state
            .commands
            .try_send(Command {
                id,
                text,
                completed,
            })
            .map_err(|_| "chat command queue full or closed")?;
        let outcome = timed(IO_TIMEOUT, async {
            result
                .await
                .map_err(|_| "chat closed before write completed".to_string())?
        })
        .await;
        if outcome.is_err() {
            self.stop();
        }
        outcome
    }
    pub fn stop(&self) {
        self.state.stop();
    }
    pub async fn close(&self) -> Result<()> {
        self.stop();
        let _ = self.state.done.recv().await;
        self.state
            .cleanup
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Err("chat driver ended without cleanup".into()))
    }
}
struct Driver {
    state: Arc<State>,
    commands: Receiver<Command>,
    cancel: Receiver<()>,
    done: Sender<()>,
    host_sessions: Option<Arc<Mutex<HashMap<Uuid, Connection>>>>,
    host_cleanup: Option<Arc<Mutex<Option<String>>>>,
}
impl Drop for Driver {
    fn drop(&mut self) {
        self.state.stop();
        self.commands.close();
        let result = self
            .state
            .cleanup
            .lock()
            .unwrap()
            .get_or_insert_with(|| Err("chat driver ended without cleanup".into()))
            .clone();
        if let (Some(errors), Err(error)) = (&self.host_cleanup, result) {
            errors.lock().unwrap().get_or_insert(error);
        }
        if let Some(sessions) = &self.host_sessions {
            let mut sessions = sessions.lock().unwrap();
            let _ = self.state.event("closed", None, None);
            sessions.remove(&self.state.session);
        } else {
            self.state.events.close();
        }
        self.done.close();
    }
}
fn channel(
    session: Uuid,
    peer: String,
    host_events: Option<(Sender<Event>, Receiver<Event>)>,
) -> (Connection, Driver) {
    let host = host_events.is_some();
    let (events_tx, events) = host_events.unwrap_or_else(|| async_channel::bounded(CAPACITY));
    let (commands, receiver) = async_channel::bounded(CAPACITY);
    let (cancel, cancellation) = async_channel::bounded(1);
    let (done_tx, done) = async_channel::bounded(1);
    let state = Arc::new(State {
        session,
        peer,
        host,
        paired: AtomicBool::new(false),
        approving: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        commands,
        cancel,
        done,
        cleanup: Mutex::new(None),
        events: events_tx,
    });
    (
        Connection {
            state: state.clone(),
            events,
        },
        Driver {
            state,
            commands: receiver,
            cancel: cancellation,
            done: done_tx,
            host_sessions: None,
            host_cleanup: None,
        },
    )
}

/// Opens outbound only. The caller must spawn and retain the returned driver until close.
pub async fn connect(
    protocols: AukiPeerProtocols,
    peer: PeerId,
    route: Multiaddr,
) -> Result<(Connection, impl Future<Output = ()>)> {
    let mut stream = timed(IO_TIMEOUT, async {
        protocols
            .open_exact(peer, route, PROTOCOL_ID)
            .await
            .map_err(|e| e.to_string())
    })
    .await?;
    let hello = timed(IO_TIMEOUT, read_frame(&mut stream)).await;
    let session_id = match hello {
        Ok(Frame::Pending { session_id }) => session_id,
        other => {
            let cleanup = timed(IO_TIMEOUT, async {
                stream.close().await.map_err(|e| e.to_string())
            })
            .await;
            let error = other
                .err()
                .unwrap_or_else(|| "expected pending chat hello".into());
            return Err(match cleanup {
                Ok(()) => error,
                Err(close) => format!("{error}; cleanup: {close}"),
            });
        }
    };
    let (connection, driver) = channel(session_id, peer.to_string(), None);
    Ok((connection, async move {
        run(&driver, &mut stream, false).await;
        drop(driver);
    }))
}

async fn run<S: AsyncRead + AsyncWrite + Unpin>(driver: &Driver, stream: &mut S, host: bool) {
    let (mut read, mut write) = stream.split();
    let exchange = async {
        if host {
            write_frame(
                &mut write,
                &Frame::Pending {
                    session_id: driver.state.session,
                },
            )
            .await?;
            driver.state.event("pending", None, None)?;
            // Any incoming byte before approval terminates the stream without reading a payload.
            timed(PENDING_TTL, async {
                let mut byte = [0];
                let incoming = read.read_exact(&mut byte).fuse();
                let approval = driver.commands.recv().fuse();
                pin_mut!(incoming, approval);
                futures::select_biased! {
                    _ = incoming => Err("chat data before approval".into()),
                    command = approval => {
                        let command = command.map_err(|_| "chat closed")?;
                        if !command.text.is_empty() { return Err("chat data before approval".into()); }
                        write_frame(&mut write, &Frame::Paired { session_id: driver.state.session }).await?;
                        driver.state.paired.store(true, Ordering::Release);
                        driver.state.event("paired", None, None)?;
                        let _ = command.completed.send(Ok(()));
                        Ok(())
                    }
                }
            }).await?;
        } else {
            match timed(PENDING_TTL, read_frame(&mut read)).await? {
                Frame::Paired { session_id } if session_id == driver.state.session => {}
                _ => return Err::<(), String>("invalid chat pairing".into()),
            }
            driver.state.paired.store(true, Ordering::Release);
            driver.state.event("paired", None, None)?;
        }
        let mut seen = HashSet::new();
        let mut pending = HashSet::new();
        loop {
            // Keep this future alive across outgoing commands: a partial header must never be discarded.
            let frame = read_frame(&mut read).fuse();
            pin_mut!(frame);
            loop {
                let command = driver.commands.recv().fuse();
                pin_mut!(command);
                futures::select_biased! {
                    incoming = frame => {
                        match incoming? {
                            Frame::Message { session_id, id, text } if session_id == driver.state.session => {
                                text_valid(&text)?;
                                if seen.len() >= CAPACITY || !seen.insert(id) { return Err("chat replay or session message limit".into()); }
                                driver.state.event("message", Some(id), Some(text))?;
                                write_frame(&mut write, &Frame::Ack { session_id, id }).await?;
                            },
                            Frame::Ack { session_id, id } if session_id == driver.state.session && pending.remove(&id) => {
                                driver.state.event("ack", Some(id), None)?;
                            },
                            _ => return Err("unexpected chat frame or unknown acknowledgement".into()),
                        }
                        break;
                    },
                    command = command => {
                        let command = command.map_err(|_| "chat closed")?;
                        if seen.len() >= CAPACITY || !seen.insert(command.id) {
                            let _ = command.completed.send(Err("duplicate message ID or session message limit".into()));
                            continue;
                        }
                        pending.insert(command.id);
                        let result = write_frame(&mut write, &Frame::Message { session_id: driver.state.session, id: command.id, text: command.text }).await;
                        let failed = result.is_err(); let _ = command.completed.send(result);
                        if failed { return Err("chat write failed".into()); }
                    }
                }
            }
        }
    };
    {
        let exchange = timed(SESSION_TTL, exchange).fuse();
        let cancel = driver.cancel.recv().fuse();
        pin_mut!(exchange, cancel);
        futures::select_biased! { _ = cancel => {}, _ = exchange => {} }
    }
    driver.state.stop();
    let cleanup = timed(IO_TIMEOUT, async {
        write.close().await.map_err(|_| "chat close failed".into())
    })
    .await;
    *driver.state.cleanup.lock().unwrap() = Some(cleanup);
}

struct MountGuard(HostHandle);
impl MountGuard {
    fn handle(&self) -> HostHandle {
        self.0.clone()
    }
}
impl Drop for MountGuard {
    fn drop(&mut self) {
        self.0.stop();
    }
}

pub struct Host {
    registration: Option<AukiProtocolRegistration>,
    handle: HostHandle,
}
#[derive(Clone)]
pub struct HostHandle {
    sessions: Arc<Mutex<HashMap<Uuid, Connection>>>,
    events: Receiver<Event>,
    delivery: Sender<Event>,
    closed: Arc<AtomicBool>,
    cleanup_error: Arc<Mutex<Option<String>>>,
}
impl Host {
    pub fn mount(protocols: AukiPeerProtocols) -> Result<Self> {
        let (delivery, events) = async_channel::bounded(CAPACITY);
        let handle = HostHandle {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            events,
            delivery,
            closed: Arc::new(AtomicBool::new(false)),
            cleanup_error: Arc::new(Mutex::new(None)),
        };
        let serving = MountGuard(handle.clone());
        let spec = AukiProtocolSpec::new(PROTOCOL_ID, 4, MAX_FRAME_BYTES as u32)
            .map_err(|e| e.to_string())?;
        let registration = protocols
            .register(spec, move |mut stream| {
                let serving = serving.handle();
                async move {
                    let driver = {
                        let mut sessions = serving.sessions.lock().unwrap();
                        if serving.closed.load(Ordering::Acquire) || sessions.len() >= 4 {
                            return;
                        }
                        let id = Uuid::new_v4();
                        let (connection, mut driver) = channel(
                            id,
                            stream.remote_peer().peer_id.to_string(),
                            Some((serving.delivery.clone(), serving.events.clone())),
                        );
                        driver.host_sessions = Some(serving.sessions.clone());
                        driver.host_cleanup = Some(serving.cleanup_error.clone());
                        sessions.insert(id, connection);
                        driver
                    };
                    run(&driver, &mut stream, true).await;
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            registration: Some(registration),
            handle,
        })
    }
    pub fn handle(&self) -> HostHandle {
        self.handle.clone()
    }
    pub async fn close(mut self) -> Result<()> {
        self.handle.stop();
        let connections: Vec<_> = self
            .handle
            .sessions
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect();
        let results = futures::future::join_all(connections.iter().map(Connection::close)).await;
        let registration = if let Some(registration) = self.registration.take() {
            registration.close().await.map_err(|e| e.to_string())
        } else {
            Ok(())
        };
        let mut errors: Vec<String> = results.into_iter().filter_map(Result::err).collect();
        if let Err(error) = registration {
            errors.push(error);
        }
        if let Some(error) = self.handle.cleanup_error.lock().unwrap().clone() {
            if !errors.contains(&error) {
                errors.push(error);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.handle.stop();
    }
}
impl HostHandle {
    pub fn stop(&self) {
        self.closed.store(true, Ordering::Release);
        self.delivery.close();
        for connection in self.sessions.lock().unwrap().values() {
            connection.stop();
        }
    }
    pub async fn next_event(&self) -> Result<Event> {
        if self.closed.load(Ordering::Acquire) {
            return Err("chat host closed".into());
        }
        self.events
            .recv()
            .await
            .map_err(|_| "chat host closed or event queue overloaded".into())
    }
    fn get(&self, session: &str) -> Result<Connection> {
        if self.closed.load(Ordering::Acquire) {
            return Err("chat host closed".into());
        }
        let id = Uuid::parse_str(session).map_err(|_| "invalid session UUID")?;
        self.sessions
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or_else(|| "unknown live chat session".into())
    }
    pub async fn approve(&self, session: &str, peer: &str) -> Result<()> {
        let connection = self.get(session)?;
        if connection.state.peer != peer
            || connection.state.closed.load(Ordering::Acquire)
            || connection.state.paired.load(Ordering::Acquire)
            || connection
                .state
                .approving
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return Err("approval requires exact pending session and authenticated peer".into());
        }
        let (completed, result) = oneshot::channel();
        connection
            .state
            .commands
            .try_send(Command {
                id: Uuid::nil(),
                text: String::new(),
                completed,
            })
            .map_err(|_| "chat approval queue full or closed")?;
        let outcome = timed(IO_TIMEOUT, async {
            result
                .await
                .map_err(|_| "chat closed during approval".to_string())?
        })
        .await;
        if outcome.is_err() {
            connection.stop();
        }
        outcome
    }
    pub async fn send(&self, session: &str, id: &str, text: String) -> Result<()> {
        self.get(session)?.send(id, text).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framed_message_round_trips_exact_text_and_session() {
        futures::executor::block_on(async {
            let session_id = Uuid::new_v4();
            let id = Uuid::new_v4();
            let mut wire = futures::io::Cursor::new(Vec::new());
            write_frame(
                &mut wire,
                &Frame::Message {
                    session_id,
                    id,
                    text: "different reply 🦀".into(),
                },
            )
            .await
            .unwrap();
            wire.set_position(0);
            match read_frame(&mut wire).await.unwrap() {
                Frame::Message {
                    session_id: actual_session,
                    id: actual_id,
                    text,
                } => {
                    assert_eq!(actual_session, session_id);
                    assert_eq!(actual_id, id);
                    assert_eq!(text, "different reply 🦀");
                }
                _ => panic!("wrong frame"),
            }
        });
    }
    #[test]
    fn approval_requires_live_exact_pair_and_cannot_enqueue_twice() {
        let (delivery, events) = async_channel::bounded(CAPACITY);
        let session = Uuid::new_v4();
        let (connection, driver) = channel(
            session,
            "authenticated-peer".into(),
            Some((delivery.clone(), events.clone())),
        );
        let handle = HostHandle {
            sessions: Arc::new(Mutex::new(HashMap::from([(session, connection)]))),
            events,
            delivery,
            closed: Arc::new(AtomicBool::new(false)),
            cleanup_error: Arc::new(Mutex::new(None)),
        };
        futures::executor::block_on(async {
            assert!(
                handle
                    .approve(&session.to_string(), "wrong-peer")
                    .await
                    .is_err()
            );
            assert!(
                handle
                    .approve(&Uuid::new_v4().to_string(), "authenticated-peer")
                    .await
                    .is_err()
            );
            assert!(
                handle
                    .send(
                        &session.to_string(),
                        &Uuid::new_v4().to_string(),
                        "not approved".into()
                    )
                    .await
                    .is_err()
            );
            let session_text = session.to_string();
            let first = handle.approve(&session_text, "authenticated-peer").fuse();
            pin_mut!(first);
            assert!(futures::poll!(&mut first).is_pending());
            assert!(
                handle
                    .approve(&session_text, "authenticated-peer")
                    .await
                    .is_err()
            );
            let command = driver.commands.try_recv().unwrap();
            assert!(driver.commands.try_recv().is_err());
            command.completed.send(Ok(())).unwrap();
            first.await.unwrap();
            handle.stop();
            assert!(
                handle
                    .approve(&session_text, "authenticated-peer")
                    .await
                    .is_err()
            );
        });
    }
    #[test]
    fn oversized_header_rejected_without_body() {
        let mut input = futures::io::Cursor::new(u32::MAX.to_be_bytes());
        assert_eq!(
            futures::executor::block_on(read_frame(&mut input)).unwrap_err(),
            "invalid chat frame length"
        );
    }
    #[test]
    fn utf8_limit_is_bytes_and_empty_is_invalid() {
        assert!(text_valid("").is_err());
        assert!(text_valid(&"é".repeat(1024)).is_ok());
        assert!(text_valid(&"é".repeat(1025)).is_err());
    }
    #[test]
    fn malformed_uuid_and_extra_fields_fail_closed() {
        assert!(
            serde_json::from_str::<Frame>(r#"{"type":"ack","session_id":"wrong","id":"wrong"}"#)
                .is_err()
        );
        let id = Uuid::new_v4();
        assert!(
            serde_json::from_str::<Frame>(&format!(
                r#"{{"type":"paired","session_id":"{id}","text":"smuggled"}}"#
            ))
            .is_err()
        );
    }
    #[test]
    fn closed_driver_rejects_send_and_wakes_receive_and_close() {
        let (connection, driver) = channel(Uuid::new_v4(), "peer".into(), None);
        drop(driver);
        futures::executor::block_on(async {
            assert!(connection.next_event().await.is_err());
            assert!(
                connection
                    .send(&Uuid::new_v4().to_string(), "hello".into())
                    .await
                    .is_err()
            );
            assert!(connection.close().await.is_err());
            assert!(connection.close().await.is_err());
        });
    }
    #[test]
    fn event_overload_is_observable_failure() {
        let (connection, driver) = channel(Uuid::new_v4(), "peer".into(), None);
        for _ in 0..CAPACITY {
            driver.state.event("paired", None, None).unwrap();
        }
        assert!(driver.state.event("paired", None, None).is_err());
        assert!(connection.events.is_closed());
    }
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod wire_tests;
