use serde::{Deserialize, Serialize};

pub const BROWSER_PROBE_PROTOCOL: &str = "/auki/browser-probe/0.0.1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserProbeRequest {
    pub nonce: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserProbeResponse {
    pub nonce: String,
    pub payload: Vec<u8>,
    pub responder: String,
}

impl BrowserProbeResponse {
    pub fn from_request(request: &BrowserProbeRequest, responder: impl Into<String>) -> Self {
        Self {
            nonce: request.nonce.clone(),
            payload: request.payload.clone(),
            responder: responder.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_preserves_nonce_payload_and_names_responder() {
        let request = BrowserProbeRequest {
            nonce: "probe-001".to_string(),
            payload: vec![1, 2, 3, 4],
        };

        let response = BrowserProbeResponse::from_request(&request, "native-probe");

        assert_eq!(response.nonce, "probe-001");
        assert_eq!(response.payload, vec![1, 2, 3, 4]);
        assert_eq!(response.responder, "native-probe");
    }
}
