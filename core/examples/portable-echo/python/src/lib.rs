//! Thin Python adapter for the shared portable echo endpoint.

// PyO3 0.22 macro expansions trigger these Rust 2024 and Clippy lints.
#![allow(unsafe_op_in_unsafe_fn)]
#![allow(clippy::useless_conversion)]

use auki_echo_protocol::{
    EchoClient, EchoEndpoint, EchoEventReceiver, EchoServeEvent, PROTOCOL_ID,
};
use auki_sdk_binding::{
    PyAukiPeer,
    cleanup::{DetachedCleanup, wait_cleanup},
};
use auki_sdk_rs::{Multiaddr, PeerId};
use parking_lot::Mutex;
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyModule},
};

fn runtime_error(context: &'static str, error: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(format!("{context}: {error}"))
}

fn parse_target(peer_id: &str, route: &str) -> PyResult<(PeerId, Multiaddr)> {
    let peer_id = peer_id
        .parse::<PeerId>()
        .map_err(|error| PyValueError::new_err(format!("invalid remote Peer ID: {error}")))?;
    let route = route
        .parse::<Multiaddr>()
        .map_err(|error| PyValueError::new_err(format!("invalid remote route: {error}")))?;
    Ok((peer_id, route))
}

struct EchoOwner {
    endpoint: Mutex<Option<EchoEndpoint>>,
    cleanup: DetachedCleanup,
}

impl EchoOwner {
    fn new(endpoint: EchoEndpoint) -> Self {
        Self {
            endpoint: Mutex::new(Some(endpoint)),
            cleanup: DetachedCleanup::default(),
        }
    }

    fn ensure_open(&self) -> PyResult<()> {
        if self.endpoint.lock().is_some() {
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("portable echo endpoint is stopped"))
        }
    }

    fn begin_close(
        &self,
    ) -> tokio::sync::watch::Receiver<Option<auki_sdk_binding::cleanup::CleanupResult>> {
        self.cleanup.get_or_start(|| {
            let close = self.endpoint.lock().take().map(EchoEndpoint::close);
            async move {
                match close {
                    Some(close) => close.await.map_err(|error| error.to_string()),
                    None => Ok(()),
                }
            }
        })
    }
}

impl Drop for EchoOwner {
    fn drop(&mut self) {
        let Some(endpoint) = self.endpoint.get_mut().take() else {
            return;
        };
        pyo3_async_runtimes::tokio::get_runtime().spawn(async move {
            let _ = endpoint.close().await;
        });
    }
}

/// Mounted portable echo service and outbound client.
#[pyclass(name = "AukiEcho")]
struct PyAukiEcho {
    owner: EchoOwner,
    client: EchoClient,
    events: EchoEventReceiver,
}

#[pymethods]
impl PyAukiEcho {
    /// Mount the shared Rust protocol endpoint inside the native async runtime.
    #[staticmethod]
    fn mount<'py>(py: Python<'py>, peer: &PyAukiPeer) -> PyResult<Bound<'py, PyAny>> {
        let protocols = peer.protocols();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let endpoint = EchoEndpoint::mount(protocols)
                .map_err(|error| runtime_error("mount portable echo", error))?;
            let client = endpoint.client();
            let events = endpoint.events();
            Python::with_gil(|py| {
                Py::new(
                    py,
                    Self {
                        owner: EchoOwner::new(endpoint),
                        client,
                        events,
                    },
                )
            })
        })
    }

    #[getter]
    fn protocol(&self) -> &'static str {
        PROTOCOL_ID
    }

    /// Send one bounded echo through an exact authenticated route.
    fn send_exact<'py>(
        &self,
        py: Python<'py>,
        remote_peer_id: String,
        route: String,
        payload: &Bound<'_, PyBytes>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.owner.ensure_open()?;
        let (remote_peer_id, route) = parse_target(&remote_peer_id, &route)?;
        let payload = payload.as_bytes().to_vec();
        let client = self.client.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let receipt = client
                .send_exact(remote_peer_id, route, payload)
                .await
                .map_err(|error| runtime_error("run portable echo", error))?;
            Python::with_gil(|py| {
                Py::new(
                    py,
                    PyEchoReceipt {
                        remote_peer_id: receipt.remote_peer_id.to_string(),
                        payload: receipt.payload,
                    },
                )
            })
        })
    }

    /// Wait for one inbound completion from the bounded Rust event queue.
    fn next_served<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.owner.ensure_open()?;
        let events = self.events.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let receipt = match events.recv().await {
                Some(EchoServeEvent::Served(receipt)) => receipt,
                Some(EchoServeEvent::Failed {
                    remote_peer_id,
                    error,
                }) => {
                    return Err(runtime_error(
                        "serve portable echo",
                        format!("remote peer {remote_peer_id}: {error}"),
                    ));
                }
                Some(EchoServeEvent::Lagged { dropped }) => {
                    return Err(runtime_error(
                        "observe portable echo",
                        format!("event consumer fell behind by {dropped} events"),
                    ));
                }
                None => return Err(PyRuntimeError::new_err("portable echo endpoint is stopped")),
            };
            Python::with_gil(|py| {
                Py::new(
                    py,
                    PyEchoReceipt {
                        remote_peer_id: receipt.remote_peer_id.to_string(),
                        payload: receipt.payload,
                    },
                )
            })
        })
    }

    /// Stop inbound serving behind one detached, replayable cleanup barrier.
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let cleanup = self.owner.begin_close();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            wait_cleanup(cleanup)
                .await
                .map_err(|error| runtime_error("close portable echo", error))
        })
    }
}

/// One validated echo response or inbound request.
#[pyclass(name = "EchoReceipt", frozen)]
struct PyEchoReceipt {
    remote_peer_id: String,
    payload: Vec<u8>,
}

#[pymethods]
impl PyEchoReceipt {
    #[getter]
    fn remote_peer_id(&self) -> String {
        self.remote_peer_id.clone()
    }

    #[getter]
    fn payload(&self, py: Python<'_>) -> Py<PyBytes> {
        PyBytes::new_bound(py, &self.payload).unbind()
    }

    fn __repr__(&self) -> String {
        format!(
            "EchoReceipt(remote_peer_id={:?}, payload_len={})",
            self.remote_peer_id,
            self.payload.len()
        )
    }
}

#[pymodule]
fn auki_portable_echo(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    auki_sdk_binding::register_facade(module)?;
    module.add_class::<PyAukiEcho>()?;
    module.add_class::<PyEchoReceipt>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_module_exposes_peer_and_echo_facades() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_portable_echo").unwrap();
            auki_sdk_binding::register_facade(&module).unwrap();
            module.add_class::<PyAukiEcho>().unwrap();
            module.add_class::<PyEchoReceipt>().unwrap();
            for name in [
                "AukiSession",
                "AukiDomain",
                "AukiDiscoveryCandidate",
                "AukiPeer",
                "AukiEcho",
                "EchoReceipt",
            ] {
                assert!(module.getattr(name).is_ok(), "missing {name}");
            }
        });
    }

    #[test]
    fn exact_target_validation_fails_at_the_binding_boundary() {
        Python::with_gil(|_py| {
            assert!(parse_target("not-a-peer", "/ip4/127.0.0.1/tcp/1").is_err());
            assert!(
                parse_target(
                    "12D3KooWJ5Xw8jCxxbVZXcaUpf7h8fWgpcnH9tGgNfZQ1nSJXUL3",
                    "not-a-route"
                )
                .is_err()
            );
        });
    }

    #[test]
    fn receipt_repr_does_not_dump_payload_bytes() {
        let receipt = PyEchoReceipt {
            remote_peer_id: "peer".into(),
            payload: vec![0xde, 0xad, 0xbe, 0xef],
        };
        assert_eq!(
            receipt.__repr__(),
            "EchoReceipt(remote_peer_id=\"peer\", payload_len=4)"
        );
    }
}
