//! Operator-controlled Chat on the combined native SDK module.
use crate::runtime_error;
use auki_echo_protocol::chat::{Host, HostHandle, PROTOCOL_ID};
use auki_sdk_binding::{
    PyAukiPeer,
    cleanup::{DetachedCleanup, wait_cleanup},
};
use parking_lot::Mutex;
use pyo3::prelude::*;

#[pyclass(name = "AukiChatHost")]
pub(crate) struct PyAukiChatHost {
    host: Mutex<Option<Host>>,
    handle: HostHandle,
    cleanup: DetachedCleanup,
}
#[pymethods]
impl PyAukiChatHost {
    #[staticmethod]
    fn mount<'py>(py: Python<'py>, peer: &PyAukiPeer) -> PyResult<Bound<'py, PyAny>> {
        let protocols = peer.protocols();
        auki_sdk_binding::async_completion::future_into_py(py, async move {
            let host =
                Host::mount(protocols).map_err(|error| runtime_error("mount Chat", error))?;
            let handle = host.handle();
            Python::with_gil(|py| {
                Py::new(
                    py,
                    Self {
                        host: Mutex::new(Some(host)),
                        handle,
                        cleanup: DetachedCleanup::default(),
                    },
                )
            })
        })
    }
    #[getter]
    fn protocol(&self) -> &'static str {
        PROTOCOL_ID
    }
    fn next_event<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let handle = self.handle.clone();
        auki_sdk_binding::async_completion::future_into_py(py, async move {
            handle
                .next_event()
                .await
                .and_then(|event| event.json())
                .map_err(|error| runtime_error("observe Chat", error))
        })
    }
    fn approve<'py>(
        &self,
        py: Python<'py>,
        session_id: String,
        peer_id: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let handle = self.handle.clone();
        auki_sdk_binding::async_completion::future_into_py(py, async move {
            handle
                .approve(&session_id, &peer_id)
                .await
                .map_err(|error| runtime_error("approve Chat", error))
        })
    }
    fn send<'py>(
        &self,
        py: Python<'py>,
        session_id: String,
        id: String,
        text: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let handle = self.handle.clone();
        auki_sdk_binding::async_completion::future_into_py(py, async move {
            handle
                .send(&session_id, &id, text)
                .await
                .map_err(|error| runtime_error("send Chat", error))
        })
    }
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.handle.stop();
        let cleanup = self.cleanup.get_or_start(|| {
            let host = self.host.lock().take();
            async move {
                match host {
                    Some(host) => host.close().await,
                    None => Ok(()),
                }
            }
        });
        auki_sdk_binding::async_completion::future_into_py(py, async move {
            wait_cleanup(cleanup)
                .await
                .map_err(|error| runtime_error("close Chat", error))
        })
    }
}
impl Drop for PyAukiChatHost {
    fn drop(&mut self) {
        self.handle.stop();
        if let Some(host) = self.host.get_mut().take() {
            pyo3_async_runtimes::tokio::get_runtime().spawn(async move {
                let _ = host.close().await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn combined_facade_exports_operator_chat_contract() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_portable_echo").unwrap();
            crate::auki_portable_echo(py, &module).unwrap();
            assert!(module.getattr("AukiPeer").is_ok());
            let host = module.getattr("AukiChatHost").unwrap();
            for name in [
                "mount",
                "protocol",
                "next_event",
                "approve",
                "send",
                "close",
            ] {
                assert!(host.getattr(name).is_ok(), "missing {name}");
            }
        });
    }
}
