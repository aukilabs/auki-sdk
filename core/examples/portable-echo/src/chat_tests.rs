//! Full driver tests over a bounded, portable, in-memory duplex stream.
use super::*;
use std::{
    collections::VecDeque,
    pin::Pin,
    task::{Context, Poll, Waker},
};
#[derive(Default)]
struct Pipe {
    bytes: VecDeque<u8>,
    closed: bool,
    reader: Option<Waker>,
}
struct Duplex {
    input: Arc<Mutex<Pipe>>,
    output: Arc<Mutex<Pipe>>,
    fail_close: bool,
}
fn duplex() -> (Duplex, Duplex) {
    let a = Arc::new(Mutex::new(Pipe::default()));
    let b = Arc::new(Mutex::new(Pipe::default()));
    (
        Duplex {
            input: a.clone(),
            output: b.clone(),
            fail_close: false,
        },
        Duplex {
            input: b,
            output: a,
            fail_close: false,
        },
    )
}
impl AsyncRead for Duplex {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        let mut p = self.input.lock().unwrap();
        if p.bytes.is_empty() && !p.closed {
            p.reader = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let n = buf.len().min(p.bytes.len());
        for byte in &mut buf[..n] {
            *byte = p.bytes.pop_front().unwrap();
        }
        Poll::Ready(Ok(n))
    }
}
impl AsyncWrite for Duplex {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let mut p = self.output.lock().unwrap();
        assert!(
            p.bytes.len() + buf.len() <= MAX_FRAME_BYTES * 2,
            "bounded test transport overflow"
        );
        p.bytes.extend(buf);
        if let Some(waker) = p.reader.take() {
            waker.wake();
        }
        Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let mut p = self.output.lock().unwrap();
        p.closed = true;
        if let Some(waker) = p.reader.take() {
            waker.wake();
        }
        Poll::Ready(if self.fail_close {
            Err(std::io::Error::other("injected close failure"))
        } else {
            Ok(())
        })
    }
}
fn host() -> (Connection, Driver, HostHandle) {
    let (delivery, events) = async_channel::bounded(CAPACITY);
    let session = Uuid::new_v4();
    let (connection, mut driver) = channel(
        session,
        "authenticated-peer".into(),
        Some((delivery.clone(), events.clone())),
    );
    let handle = HostHandle {
        sessions: Arc::new(Mutex::new(HashMap::from([(session, connection.clone())]))),
        events,
        delivery,
        closed: Arc::new(AtomicBool::new(false)),
        cleanup_error: Arc::new(Mutex::new(None)),
    };
    driver.host_sessions = Some(handle.sessions.clone());
    driver.host_cleanup = Some(handle.cleanup_error.clone());
    (connection, driver, handle)
}
fn bounded(work: impl Future<Output = ()>) {
    futures::executor::block_on(async {
        timed(Duration::from_secs(2), async {
            work.await;
            Ok(())
        })
        .await
        .unwrap()
    });
}
async fn pair(handle: &HostHandle, remote: &mut Duplex, session: Uuid) {
    assert!(matches!(
        read_frame(remote).await.unwrap(),
        Frame::Pending { .. }
    ));
    assert_eq!(handle.next_event().await.unwrap().kind, "pending");
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
    handle
        .approve(&session.to_string(), "authenticated-peer")
        .await
        .unwrap();
    assert!(matches!(
        read_frame(remote).await.unwrap(),
        Frame::Paired { .. }
    ));
    assert_eq!(handle.next_event().await.unwrap().kind, "paired");
    assert!(
        handle
            .approve(&session.to_string(), "authenticated-peer")
            .await
            .is_err()
    );
}
#[test]
fn wire_preapproval_byte_closes_without_message_and_releases_slot_after_closed() {
    bounded(async {
        let (connection, driver, handle) = host();
        let (mut local, mut remote) = duplex();
        let task = async {
            run(&driver, &mut local, true).await;
            drop(driver);
        };
        let client = async {
            read_frame(&mut remote).await.unwrap();
            assert_eq!(handle.next_event().await.unwrap().kind, "pending");
            remote.write_all(&[0]).await.unwrap();
            assert_eq!(handle.next_event().await.unwrap().kind, "closed");
            assert!(handle.sessions.lock().unwrap().is_empty());
            assert!(handle.events.try_recv().is_err());
            connection.close().await.unwrap();
        };
        futures::join!(task, client);
    });
}
#[test]
fn wire_unknown_wrong_session_and_repeated_ack_close() {
    for kind in 0..3 {
        bounded(async {
            let (connection, driver, handle) = host();
            let session = connection.state.session;
            let (mut local, mut remote) = duplex();
            let task = async {
                run(&driver, &mut local, true).await;
                drop(driver);
            };
            let client = async {
                pair(&handle, &mut remote, session).await;
                let id = Uuid::new_v4();
                if kind != 0 {
                    connection
                        .send(&id.to_string(), "outgoing".into())
                        .await
                        .unwrap();
                    assert!(matches!(
                        read_frame(&mut remote).await.unwrap(),
                        Frame::Message { .. }
                    ));
                }
                if kind == 2 {
                    write_frame(
                        &mut remote,
                        &Frame::Ack {
                            session_id: session,
                            id,
                        },
                    )
                    .await
                    .unwrap();
                    assert_eq!(handle.next_event().await.unwrap().kind, "ack");
                }
                write_frame(
                    &mut remote,
                    &Frame::Ack {
                        session_id: if kind == 1 { Uuid::new_v4() } else { session },
                        id,
                    },
                )
                .await
                .unwrap();
                assert_eq!(handle.next_event().await.unwrap().kind, "closed");
                connection.close().await.unwrap();
            };
            futures::join!(task, client);
        });
    }
}
#[test]
fn wire_cross_direction_id_reuse_is_rejected_in_both_directions() {
    for inbound_first in [true, false] {
        bounded(async {
            let (connection, driver, handle) = host();
            let session = connection.state.session;
            let (mut local, mut remote) = duplex();
            let task = async {
                run(&driver, &mut local, true).await;
                drop(driver);
            };
            let client = async {
                pair(&handle, &mut remote, session).await;
                let id = Uuid::new_v4();
                if inbound_first {
                    write_frame(
                        &mut remote,
                        &Frame::Message {
                            session_id: session,
                            id,
                            text: "incoming".into(),
                        },
                    )
                    .await
                    .unwrap();
                    assert_eq!(handle.next_event().await.unwrap().kind, "message");
                    read_frame(&mut remote).await.unwrap();
                    assert!(
                        connection
                            .send(&id.to_string(), "collision".into())
                            .await
                            .is_err()
                    );
                } else {
                    connection
                        .send(&id.to_string(), "outgoing".into())
                        .await
                        .unwrap();
                    read_frame(&mut remote).await.unwrap();
                    write_frame(
                        &mut remote,
                        &Frame::Message {
                            session_id: session,
                            id,
                            text: "collision".into(),
                        },
                    )
                    .await
                    .unwrap();
                }
                assert_eq!(handle.next_event().await.unwrap().kind, "closed");
                connection.close().await.unwrap();
            };
            futures::join!(task, client);
        });
    }
}
#[test]
fn wire_combined_capacity_is_128() {
    bounded(async {
        let (connection, driver, handle) = host();
        let session = connection.state.session;
        let (mut local, mut remote) = duplex();
        let task = async {
            run(&driver, &mut local, true).await;
            drop(driver);
        };
        let client = async {
            pair(&handle, &mut remote, session).await;
            for i in 0..CAPACITY {
                let id = Uuid::new_v4();
                if i % 2 == 0 {
                    connection
                        .send(&id.to_string(), "out".into())
                        .await
                        .unwrap();
                    read_frame(&mut remote).await.unwrap();
                    write_frame(
                        &mut remote,
                        &Frame::Ack {
                            session_id: session,
                            id,
                        },
                    )
                    .await
                    .unwrap();
                    assert_eq!(handle.next_event().await.unwrap().kind, "ack");
                } else {
                    write_frame(
                        &mut remote,
                        &Frame::Message {
                            session_id: session,
                            id,
                            text: "in".into(),
                        },
                    )
                    .await
                    .unwrap();
                    assert_eq!(handle.next_event().await.unwrap().kind, "message");
                    read_frame(&mut remote).await.unwrap();
                }
            }
            write_frame(
                &mut remote,
                &Frame::Message {
                    session_id: session,
                    id: Uuid::new_v4(),
                    text: "129".into(),
                },
            )
            .await
            .unwrap();
            assert_eq!(handle.next_event().await.unwrap().kind, "closed");
            connection.close().await.unwrap();
        };
        futures::join!(task, client);
    });
}
#[test]
fn wire_close_interrupts_pending_read_and_replays_cleanup_failure() {
    bounded(async {
        let (connection, driver) = channel(Uuid::new_v4(), "peer".into(), None);
        let (mut local, mut remote) = duplex();
        local.fail_close = true;
        let task = async {
            run(&driver, &mut local, false).await;
            drop(driver);
        };
        let client = async {
            write_frame(
                &mut remote,
                &Frame::Paired {
                    session_id: connection.state.session,
                },
            )
            .await
            .unwrap();
            assert_eq!(connection.next_event().await.unwrap().kind, "paired");
            let receive = connection.next_event().fuse();
            pin_mut!(receive);
            assert!(futures::poll!(&mut receive).is_pending());
            let first = connection.close().await.unwrap_err();
            assert_eq!(first, "chat close failed");
            assert_eq!(connection.close().await.unwrap_err(), first);
            assert!(receive.await.is_err());
        };
        futures::join!(task, client);
    });
}
#[test]
fn wire_fragmented_header_survives_outgoing_command() {
    bounded(async {
        let (connection, driver, handle) = host();
        let session = connection.state.session;
        let (mut local, mut remote) = duplex();
        let task = async {
            run(&driver, &mut local, true).await;
            drop(driver);
        };
        let client = async {
            pair(&handle, &mut remote, session).await;
            let mut encoded = futures::io::Cursor::new(Vec::new());
            write_frame(
                &mut encoded,
                &Frame::Message {
                    session_id: session,
                    id: Uuid::new_v4(),
                    text: "fragmented".into(),
                },
            )
            .await
            .unwrap();
            let bytes = encoded.into_inner();
            remote.write_all(&bytes[..2]).await.unwrap();
            connection
                .send(&Uuid::new_v4().to_string(), "interleaved".into())
                .await
                .unwrap();
            assert!(matches!(
                read_frame(&mut remote).await.unwrap(),
                Frame::Message { .. }
            ));
            remote.write_all(&bytes[2..]).await.unwrap();
            let event = handle.next_event().await.unwrap();
            assert_eq!(event.kind, "message");
            assert_eq!(event.text.as_deref(), Some("fragmented"));
            assert!(matches!(
                read_frame(&mut remote).await.unwrap(),
                Frame::Ack { .. }
            ));
            connection.close().await.unwrap();
        };
        futures::join!(task, client);
    });
}

#[test]
fn host_close_attempts_all_stream_cleanups_and_retains_prior_failures() {
    bounded(async {
        let (first, first_driver, handle) = host();
        let (mut first_stream, _first_remote) = duplex();
        first_stream.fail_close = true;
        let id = Uuid::new_v4();
        let (second, mut second_driver) = channel(
            id,
            "second-peer".into(),
            Some((handle.delivery.clone(), handle.events.clone())),
        );
        second_driver.host_sessions = Some(handle.sessions.clone());
        second_driver.host_cleanup = Some(handle.cleanup_error.clone());
        handle.sessions.lock().unwrap().insert(id, second.clone());
        let (mut second_stream, _second_remote) = duplex();
        let host = Host {
            registration: None,
            handle: handle.clone(),
        };
        let first_task = async {
            run(&first_driver, &mut first_stream, true).await;
            drop(first_driver);
        };
        let second_task = async {
            run(&second_driver, &mut second_stream, true).await;
            drop(second_driver);
        };
        let closing = async {
            let error = host.close().await.unwrap_err();
            assert!(error.contains("chat close failed"));
            assert_eq!(first.close().await.unwrap_err(), "chat close failed");
            second.close().await.unwrap();
            assert!(handle.sessions.lock().unwrap().is_empty());
            // Once failed sessions have left admission tracking, host completion still fails.
            let host = Host {
                registration: None,
                handle,
            };
            assert!(
                host.close()
                    .await
                    .unwrap_err()
                    .contains("chat close failed")
            );
        };
        futures::join!(first_task, second_task, closing);
    });
}

#[test]
fn wire_closed_events_precede_reused_admission_pending_events() {
    bounded(async {
        let (first, driver, handle) = host();
        let (mut local, _remote) = duplex();
        first.stop();
        run(&driver, &mut local, true).await;
        drop(driver);
        for _ in 0..6 {
            assert!(handle.sessions.lock().unwrap().is_empty());
            let id = Uuid::new_v4();
            let (connection, mut driver) = channel(
                id,
                "next-peer".into(),
                Some((handle.delivery.clone(), handle.events.clone())),
            );
            driver.host_sessions = Some(handle.sessions.clone());
            driver.host_cleanup = Some(handle.cleanup_error.clone());
            handle
                .sessions
                .lock()
                .unwrap()
                .insert(id, connection.clone());
            let (mut local, mut remote) = duplex();
            let task = async {
                run(&driver, &mut local, true).await;
                drop(driver);
            };
            let client = async {
                read_frame(&mut remote).await.unwrap();
                assert_eq!(handle.next_event().await.unwrap().kind, "closed");
                let pending = handle.next_event().await.unwrap();
                assert_eq!(pending.kind, "pending");
                assert_eq!(pending.session_id.as_deref(), Some(id.to_string().as_str()));
                connection.close().await.unwrap();
            };
            futures::join!(task, client);
        }
        assert_eq!(handle.next_event().await.unwrap().kind, "closed");
    });
}
