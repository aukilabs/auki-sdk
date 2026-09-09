use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use auki_portable_echo::{
    EchoProtocolError, EchoRequest, MAX_FRAME_BYTES, PROTOCOL_ID, run_client, run_server,
};
use futures::{AsyncRead, AsyncWrite, executor::block_on};

const PING_FRAME: &[u8] = b"\x00\x00\x00\x04ping";

#[test]
fn protocol_id_and_ping_wire_bytes_are_locked() {
    assert_eq!(PROTOCOL_ID, "/example/echo/1.0.0");

    let request = EchoRequest::new(b"ping".to_vec()).unwrap();
    let mut stream = ScriptedStream {
        read: PING_FRAME.to_vec(),
        ..Default::default()
    };
    let response = block_on(run_client(&mut stream, request)).unwrap();

    assert_eq!(response.as_bytes(), b"ping");
    assert_eq!(stream.written, PING_FRAME);
}

#[test]
fn server_echoes_arbitrary_binary_payload_exactly_once() {
    let payload = [0x00, 0xff, 0x10, 0x80];
    let mut stream = ScriptedStream {
        read: framed(&payload),
        ..Default::default()
    };

    let request = block_on(run_server(&mut stream)).unwrap();

    assert_eq!(request.as_bytes(), payload);
    assert_eq!(stream.written, framed(&payload));
    assert_eq!(stream.flushes, 1);
}

#[test]
fn client_rejects_a_response_that_does_not_match_its_request() {
    let request = EchoRequest::new(b"ping".to_vec()).unwrap();
    let mut stream = ScriptedStream {
        read: framed(b"pong"),
        ..Default::default()
    };

    let error = block_on(run_client(&mut stream, request)).unwrap_err();

    assert!(matches!(error, EchoProtocolError::ResponseMismatch));
    assert_eq!(stream.written, PING_FRAME);
}

#[test]
fn empty_request_is_rejected_before_a_stream_is_needed() {
    assert!(matches!(
        EchoRequest::new(Vec::new()),
        Err(EchoProtocolError::EmptyFrame)
    ));
}

#[test]
fn oversized_request_is_rejected_before_a_stream_is_needed() {
    let error = EchoRequest::new(vec![0_u8; MAX_FRAME_BYTES + 1]).unwrap_err();
    assert!(matches!(
        error,
        EchoProtocolError::FrameTooLarge {
            actual,
            maximum: MAX_FRAME_BYTES,
        } if actual == (MAX_FRAME_BYTES + 1) as u64
    ));
}

#[test]
fn empty_declared_frame_is_rejected_before_payload_read() {
    let mut stream = ScriptedStream {
        read: 0_u32.to_be_bytes().to_vec(),
        fail_reads_after: Some(4),
        ..Default::default()
    };

    let error = block_on(run_server(&mut stream)).unwrap_err();

    assert!(matches!(error, EchoProtocolError::EmptyFrame));
    assert!(stream.written.is_empty());
}

#[test]
fn oversized_declared_frame_is_rejected_before_payload_allocation_or_read() {
    let declared = (MAX_FRAME_BYTES as u32) + 1;
    let mut stream = ScriptedStream {
        read: declared.to_be_bytes().to_vec(),
        fail_reads_after: Some(4),
        ..Default::default()
    };

    let error = block_on(run_server(&mut stream)).unwrap_err();

    assert!(matches!(
        error,
        EchoProtocolError::FrameTooLarge {
            actual,
            maximum: MAX_FRAME_BYTES,
        } if actual == u64::from(declared)
    ));
    assert!(stream.written.is_empty());
}

fn framed(payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

#[derive(Default)]
struct ScriptedStream {
    read: Vec<u8>,
    read_offset: usize,
    written: Vec<u8>,
    flushes: usize,
    fail_reads_after: Option<usize>,
}

impl AsyncRead for ScriptedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        if self
            .fail_reads_after
            .is_some_and(|maximum| self.read_offset >= maximum)
        {
            return Poll::Ready(Err(io::Error::other("unexpected payload read")));
        }
        if self.read_offset == self.read.len() {
            return Poll::Ready(Ok(0));
        }
        let remaining = &self.read[self.read_offset..];
        let count = remaining.len().min(buffer.len());
        buffer[..count].copy_from_slice(&remaining[..count]);
        self.read_offset += count;
        Poll::Ready(Ok(count))
    }
}

impl AsyncWrite for ScriptedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.written.extend_from_slice(buffer);
        Poll::Ready(Ok(buffer.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.flushes += 1;
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
