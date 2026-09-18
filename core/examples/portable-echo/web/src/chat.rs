//! Chat compiled into the same module as the SDK Peer facade.
use crate::{CloseBarrier, js_error};
use auki_portable_echo::chat::{self, Connection};
use auki_sdk::{Multiaddr, PeerId};
use auki_sdk_web::AukiPeer;
use futures::{FutureExt, pin_mut};
use js_sys::Promise;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, future_to_promise, spawn_local};

#[wasm_bindgen]
pub struct AukiChatConnection {
    connection: Connection,
    closing: CloseBarrier,
}
#[wasm_bindgen]
impl AukiChatConnection {
    pub async fn connect(
        peer: &AukiPeer,
        remote_peer_id: String,
        wss_route: String,
    ) -> Result<AukiChatConnection, JsValue> {
        let protocols = peer
            .protocols()
            .ok_or_else(|| js_error("Auki peer stopped"))?;
        let remote = remote_peer_id
            .parse::<PeerId>()
            .map_err(|error| js_error(error.to_string()))?;
        let route = wss_route
            .parse::<Multiaddr>()
            .map_err(|_| js_error("invalid WSS route"))?;
        let stopped = JsFuture::from(peer.wait_stopped()).fuse();
        let opening = chat::connect(protocols, remote, route).fuse();
        pin_mut!(stopped, opening);
        let (connection, driver) = futures::select_biased! {
            _ = stopped => return Err(js_error("Auki peer stopped while opening Chat")),
            result = opening => result.map_err(js_error)?,
        };
        let stopped = JsFuture::from(peer.wait_stopped());
        let lifecycle = connection.clone();
        spawn_local(async move {
            let driver = driver.fuse();
            let stopped = stopped.fuse();
            pin_mut!(driver, stopped);
            futures::select_biased! {
                _ = stopped => { lifecycle.stop(); driver.await; },
                _ = driver => {},
            }
        });
        Ok(Self {
            connection,
            closing: CloseBarrier::default(),
        })
    }
    #[wasm_bindgen(getter, js_name = sessionId)]
    pub fn session_id(&self) -> String {
        self.connection.session_id()
    }
    #[wasm_bindgen(js_name = nextEvent)]
    pub async fn next_event(&self) -> Result<String, JsValue> {
        self.connection
            .next_event()
            .await
            .and_then(|event| event.json())
            .map_err(js_error)
    }
    pub async fn send(&self, id: String, text: String) -> Result<(), JsValue> {
        self.connection.send(&id, text).await.map_err(js_error)
    }
    #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
    pub fn close(&self) -> Promise {
        self.closing.get_or_start(|| {
            self.connection.stop();
            let connection = self.connection.clone();
            future_to_promise(async move {
                connection.close().await.map_err(js_error)?;
                Ok(JsValue::UNDEFINED)
            })
        })
    }
}
impl Drop for AukiChatConnection {
    fn drop(&mut self) {
        self.connection.stop();
    }
}
