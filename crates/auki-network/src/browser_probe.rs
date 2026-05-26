use std::time::Duration;

use futures::{AsyncReadExt, AsyncWriteExt, StreamExt as _};
use libp2p::{
    Multiaddr, StreamProtocol, Swarm, SwarmBuilder,
    swarm::{NetworkBehaviour, SwarmEvent},
};
use thiserror::Error;

use crate::{
    BROWSER_PROBE_PROTOCOL, BrowserProbeRequest, BrowserProbeResponse, PeerIdentity,
    swarm::webrtc_direct_transport,
};

pub fn responder_label(identity: &PeerIdentity) -> String {
    format!("native:{}", identity.peer_id())
}

#[derive(NetworkBehaviour)]
pub struct BrowserProbeBehaviour {
    pub stream: libp2p_stream::Behaviour,
}

#[derive(Debug, Error)]
pub enum BrowserProbeError {
    #[error("transport setup failed: {0}")]
    Transport(String),
    #[error("listen failed for {addr}: {source}")]
    Listen {
        addr: Multiaddr,
        source: libp2p::TransportError<std::io::Error>,
    },
    #[error("listener did not produce a dialable address within {0:?}")]
    ListenTimeout(Duration),
    #[error("probe protocol setup failed: {0}")]
    ProtocolSetup(String),
}

#[derive(Debug, Error)]
enum BrowserProbeStreamError {
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    #[error("encode: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("decode: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("frame is empty")]
    EmptyFrame,
    #[error("frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

pub fn build_browser_probe_swarm(
    identity: &PeerIdentity,
) -> Result<Swarm<BrowserProbeBehaviour>, BrowserProbeError> {
    SwarmBuilder::with_existing_identity(identity.keypair().clone())
        .with_tokio()
        .with_other_transport(webrtc_direct_transport)
        .map_err(|err| BrowserProbeError::Transport(err.to_string()))?
        .with_behaviour(|_| BrowserProbeBehaviour {
            stream: libp2p_stream::Behaviour::new(),
        })
        .map_err(|err| BrowserProbeError::Transport(err.to_string()))
        .map(|builder| builder.build())
}

pub async fn listen_and_serve(
    identity: PeerIdentity,
    listen_addr: Multiaddr,
) -> Result<(), BrowserProbeError> {
    let mut swarm = build_browser_probe_swarm(&identity)?;
    let mut control = swarm.behaviour().stream.new_control();
    let protocol = StreamProtocol::try_from_owned(BROWSER_PROBE_PROTOCOL.to_string())
        .map_err(|err| BrowserProbeError::ProtocolSetup(err.to_string()))?;
    let mut incoming = control
        .accept(protocol)
        .map_err(|_| BrowserProbeError::ProtocolSetup("probe protocol already registered".into()))?
        .boxed();
    swarm
        .listen_on(listen_addr.clone())
        .map_err(|source| BrowserProbeError::Listen {
            addr: listen_addr,
            source,
        })?;

    loop {
        tokio::select! {
            event = swarm.next() => {
                let Some(event) = event else { return Ok(()); };
                if let SwarmEvent::NewListenAddr { address, .. } = event {
                    println!(
                        "PARK_BROWSER_PROBE_ADDR={address}/p2p/{}",
                        identity.peer_id()
                    );
                }
            }
            inbound = incoming.next() => {
                let Some((_peer, substream)) = inbound else { return Ok(()); };
                let label = responder_label(&identity);
                tokio::spawn(async move {
                    if let Err(err) = handle_probe_stream(substream, label).await {
                        eprintln!("auki-network browser probe stream failed: {err}");
                    }
                });
            }
        }
    }
}

async fn handle_probe_stream(
    mut stream: libp2p::Stream,
    responder: String,
) -> Result<(), BrowserProbeStreamError> {
    let request = read_probe_request(&mut stream).await?;
    let response = BrowserProbeResponse::from_request(&request, responder);
    write_probe_response(&mut stream, &response).await
}

async fn read_probe_request<S>(
    stream: &mut S,
) -> Result<BrowserProbeRequest, BrowserProbeStreamError>
where
    S: AsyncReadExt + Unpin,
{
    read_json(stream).await
}

async fn write_probe_response<S>(
    stream: &mut S,
    response: &BrowserProbeResponse,
) -> Result<(), BrowserProbeStreamError>
where
    S: AsyncWriteExt + Unpin,
{
    write_json(stream, response).await
}

#[cfg(test)]
async fn write_probe_request<S>(
    stream: &mut S,
    request: &BrowserProbeRequest,
) -> Result<(), BrowserProbeStreamError>
where
    S: AsyncWriteExt + Unpin,
{
    write_json(stream, request).await
}

#[cfg(test)]
async fn read_probe_response<S>(
    stream: &mut S,
) -> Result<BrowserProbeResponse, BrowserProbeStreamError>
where
    S: AsyncReadExt + Unpin,
{
    read_json(stream).await
}

async fn write_json<S, T>(stream: &mut S, msg: &T) -> Result<(), BrowserProbeStreamError>
where
    S: AsyncWriteExt + Unpin,
    T: serde::Serialize,
{
    let bytes = serde_json::to_vec(msg).map_err(BrowserProbeStreamError::Encode)?;
    if bytes.len() as u64 > MAX_BROWSER_PROBE_FRAME_BYTES as u64 {
        return Err(BrowserProbeStreamError::FrameTooLarge {
            actual: bytes.len() as u64,
            max: MAX_BROWSER_PROBE_FRAME_BYTES as u64,
        });
    }
    let len = bytes.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(BrowserProbeStreamError::Io)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(BrowserProbeStreamError::Io)?;
    stream.flush().await.map_err(BrowserProbeStreamError::Io)?;
    Ok(())
}

async fn read_json<S, T>(stream: &mut S) -> Result<T, BrowserProbeStreamError>
where
    S: AsyncReadExt + Unpin,
    T: for<'de> serde::Deserialize<'de>,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(BrowserProbeStreamError::Io)?;
    let len = u32::from_be_bytes(len_buf);
    if len == 0 {
        return Err(BrowserProbeStreamError::EmptyFrame);
    }
    if len > MAX_BROWSER_PROBE_FRAME_BYTES {
        return Err(BrowserProbeStreamError::FrameTooLarge {
            actual: len as u64,
            max: MAX_BROWSER_PROBE_FRAME_BYTES as u64,
        });
    }
    let mut payload = vec![0u8; len as usize];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(BrowserProbeStreamError::Io)?;
    serde_json::from_slice(&payload).map_err(BrowserProbeStreamError::Decode)
}

const MAX_BROWSER_PROBE_FRAME_BYTES: u32 = 64 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responder_label_uses_native_peer_id() {
        let identity = PeerIdentity::from_seed(&[41u8; 32]);

        assert_eq!(
            responder_label(&identity),
            "native:12D3KooWSfMx5BpXVMrzyfMGHVLQe6UWNWX13ZBPLDmVoAKZ4oun"
        );
    }

    #[test]
    fn response_uses_native_responder_label() {
        let identity = PeerIdentity::from_seed(&[41u8; 32]);
        let request = BrowserProbeRequest {
            nonce: "n".to_string(),
            payload: vec![9],
        };

        let response = BrowserProbeResponse::from_request(&request, responder_label(&identity));

        assert_eq!(response.responder, responder_label(&identity));
    }

    #[test]
    fn browser_probe_swarm_uses_sdk_peer_identity() {
        let identity = PeerIdentity::from_seed(&[41u8; 32]);
        let swarm = build_browser_probe_swarm(&identity).expect("probe swarm builds");

        assert_eq!(*swarm.local_peer_id(), identity.peer_id());
    }

    #[tokio::test]
    async fn probe_stream_round_trips_length_prefixed_json() {
        let request = BrowserProbeRequest {
            nonce: "probe-001".to_string(),
            payload: vec![1, 2, 3, 4],
        };
        let response = BrowserProbeResponse::from_request(&request, "native-probe");

        let mut request_buf = Vec::new();
        write_probe_request(&mut request_buf, &request)
            .await
            .unwrap();
        let mut request_cursor = futures::io::Cursor::new(request_buf);
        assert_eq!(
            read_probe_request(&mut request_cursor).await.unwrap(),
            request
        );

        let mut response_buf = Vec::new();
        write_probe_response(&mut response_buf, &response)
            .await
            .unwrap();
        let mut response_cursor = futures::io::Cursor::new(response_buf);
        assert_eq!(
            read_probe_response(&mut response_cursor).await.unwrap(),
            response
        );
    }
}
