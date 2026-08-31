//! Thin browser binding for the shared portable echo adapter.

#![cfg(target_arch = "wasm32")]
#![forbid(unsafe_code)]

use std::{cell::RefCell, fmt::Display};

use auki_portable_echo_adapter::{
    EchoClient, EchoEndpoint, EchoEventReceiver, EchoServeEvent, PROTOCOL_ID,
};
use auki_sdk::{Multiaddr, PeerId};
pub use auki_sdk_web::{AukiDomain, AukiPeer, AukiUserSession};
use js_sys::{Error as JsError, Promise};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

#[wasm_bindgen]
pub struct AukiEcho {
    endpoint: RefCell<Option<EchoEndpoint>>,
    client: EchoClient,
    events: EchoEventReceiver,
}

#[wasm_bindgen]
impl AukiEcho {
    #[wasm_bindgen(constructor)]
    pub fn new(peer: &AukiPeer) -> Result<AukiEcho, JsValue> {
        let protocols = peer
            .protocols()
            .ok_or_else(|| js_error("Auki peer is stopped"))?;
        let endpoint = EchoEndpoint::mount(protocols)
            .map_err(|error| js_context("mount portable echo", error))?;
        let client = endpoint.client();
        let events = endpoint.events();
        Ok(Self {
            endpoint: RefCell::new(Some(endpoint)),
            client,
            events,
        })
    }

    #[wasm_bindgen(getter)]
    pub fn protocol(&self) -> String {
        PROTOCOL_ID.to_owned()
    }

    #[wasm_bindgen(js_name = sendExact)]
    pub async fn send_exact(
        &self,
        remote_peer_id: String,
        wss_route: String,
        payload: Vec<u8>,
    ) -> Result<EchoReceipt, JsValue> {
        if self.endpoint.borrow().is_none() {
            return Err(js_error("portable echo endpoint is stopped"));
        }
        let peer_id = remote_peer_id
            .parse::<PeerId>()
            .map_err(|error| js_context("parse remote Peer ID", error))?;
        let route = wss_route
            .parse::<Multiaddr>()
            .map_err(|error| js_context("parse remote WSS route", error))?;
        let receipt = self
            .client
            .send_exact(peer_id, route, payload)
            .await
            .map_err(|error| js_context("run portable echo", error))?;
        Ok(EchoReceipt {
            remote_peer_id: receipt.remote_peer_id.to_string(),
            payload: receipt.payload,
        })
    }

    #[wasm_bindgen(js_name = nextServed)]
    pub async fn next_served(&self) -> Result<EchoReceipt, JsValue> {
        match self.events.recv().await {
            Some(EchoServeEvent::Served(receipt)) => Ok(EchoReceipt {
                remote_peer_id: receipt.remote_peer_id.to_string(),
                payload: receipt.payload,
            }),
            Some(EchoServeEvent::Failed {
                remote_peer_id,
                error,
            }) => Err(js_error(format!(
                "serve portable echo from {remote_peer_id}: {error}"
            ))),
            Some(EchoServeEvent::Lagged { dropped }) => Err(js_error(format!(
                "portable echo event consumer fell behind by {dropped} events"
            ))),
            None => Err(js_error("portable echo endpoint is stopped")),
        }
    }

    /// Stop inbound serving and resolve after every admitted handler is gone.
    #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
    pub fn close(&self) -> Promise {
        let endpoint = self.endpoint.borrow_mut().take();
        future_to_promise(async move {
            if let Some(endpoint) = endpoint {
                endpoint
                    .close()
                    .await
                    .map_err(|error| js_context("close portable echo", error))?;
            }
            Ok(JsValue::UNDEFINED)
        })
    }
}

#[wasm_bindgen]
pub struct EchoReceipt {
    remote_peer_id: String,
    payload: Vec<u8>,
}

#[wasm_bindgen]
impl EchoReceipt {
    #[wasm_bindgen(getter, js_name = remotePeerId)]
    pub fn remote_peer_id(&self) -> String {
        self.remote_peer_id.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn payload(&self) -> Vec<u8> {
        self.payload.clone()
    }
}

fn js_context(context: &'static str, error: impl Display) -> JsValue {
    js_error(format!("{context}: {error}"))
}

fn js_error(message: impl AsRef<str>) -> JsValue {
    JsError::new(message.as_ref()).into()
}
