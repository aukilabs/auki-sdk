use std::sync::Arc;

use auki_domain_rs::{
    AuthenticatedParticipantInfo, MapCatalogProvider, ParticipantInfoProvider,
    ResourceCatalogProvider, ResourcesResponseV4, StreamProvider,
};
use parking_lot::RwLock;
use pyo3::{
    exceptions::PyRuntimeError,
    prelude::*,
    pyclass::{PyTraverseError, PyVisit},
};
use pyo3_async_runtimes::TaskLocals;

use crate::{
    streams::{self, PythonSourceRegistry},
    values::{PyParticipantInfo, map_resources, resource_entries},
};

struct ParticipantSlot {
    callback: Arc<Py<PyAny>>,
    last: RwLock<AuthenticatedParticipantInfo>,
}

struct StreamSlot {
    callback: Arc<Py<PyAny>>,
    locals: Arc<TaskLocals>,
    event_loop: Arc<Py<PyAny>>,
    context: Arc<Py<PyAny>>,
}

#[derive(Clone, Default)]
pub(crate) struct ProviderSlots {
    participant: Arc<RwLock<Option<Arc<ParticipantSlot>>>>,
    resource: Arc<RwLock<Option<Arc<Py<PyAny>>>>>,
    maps: Arc<RwLock<Option<Arc<Py<PyAny>>>>>,
    stream: Arc<RwLock<Option<Arc<StreamSlot>>>>,
    sources: PythonSourceRegistry,
}

impl ProviderSlots {
    pub(crate) fn set_participant(&self, py: Python<'_>, callback: Py<PyAny>) -> PyResult<()> {
        let initial = sample_participant(py, &callback)?;
        let previous = {
            let mut slot = self.participant.write();
            slot.replace(Arc::new(ParticipantSlot {
                callback: Arc::new(callback),
                last: RwLock::new(initial),
            }))
        };
        drop(previous);
        Ok(())
    }

    pub(crate) fn clear_participant(&self) {
        let previous = self.participant.write().take();
        drop(previous);
    }

    pub(crate) fn set_resource(&self, callback: Py<PyAny>) {
        let previous = {
            let mut slot = self.resource.write();
            slot.replace(Arc::new(callback))
        };
        drop(previous);
    }

    pub(crate) fn set_maps(&self, callback: Py<PyAny>) {
        let previous = {
            let mut slot = self.maps.write();
            slot.replace(Arc::new(callback))
        };
        drop(previous);
    }

    pub(crate) fn set_stream(&self, py: Python<'_>, callback: Py<PyAny>) -> PyResult<()> {
        let locals = pyo3_async_runtimes::tokio::get_current_locals(py).map_err(|error| {
            PyRuntimeError::new_err(format!(
                "DomainBuilder.stream_provider() must be called from a running asyncio loop: {error}"
            ))
        })?;
        let event_loop = Arc::new(locals.event_loop(py).unbind());
        let context = Arc::new(locals.context(py).unbind());
        let previous = {
            let mut slot = self.stream.write();
            slot.replace(Arc::new(StreamSlot {
                callback: Arc::new(callback),
                locals: Arc::new(locals),
                event_loop,
                context,
            }))
        };
        drop(previous);
        Ok(())
    }

    pub(crate) fn participant_provider(&self) -> Arc<dyn ParticipantInfoProvider> {
        let fallback = self
            .participant
            .read()
            .as_ref()
            .expect("participant provider requested before installation")
            .last
            .read()
            .clone();
        Arc::new(PythonParticipantInfo {
            slot: Arc::clone(&self.participant),
            fallback: RwLock::new(fallback),
        })
    }

    pub(crate) fn resource_provider(&self) -> Arc<dyn ResourceCatalogProvider> {
        Arc::new(PythonResourceCatalog {
            callback: Arc::clone(&self.resource),
        })
    }

    pub(crate) fn map_provider(&self) -> Arc<dyn MapCatalogProvider> {
        Arc::new(PythonMapCatalog {
            callback: Arc::clone(&self.maps),
        })
    }

    pub(crate) fn stream_provider(&self) -> StreamProvider {
        let slot = Arc::clone(&self.stream);
        let sources = self.sources.clone();
        Arc::new(move |requester, request| {
            let Some(slot) = slot.read().clone() else {
                return auki_domain_rs::StreamDispatch::Decline {
                    reason: auki_network::stream_protocol::DeclineReason::producer_shutting_down(),
                };
            };
            streams::dispatch_python_stream(
                Arc::clone(&slot.callback),
                Arc::clone(&slot.locals),
                sources.clone(),
                requester,
                request,
            )
        })
    }

    pub(crate) fn visit(&self, visit: &PyVisit<'_>) -> Result<(), PyTraverseError> {
        let participant = self.participant.read().clone();
        let resource = self.resource.read().clone();
        let maps = self.maps.read().clone();
        let stream = self.stream.read().clone();
        if let Some(slot) = participant {
            visit.call(slot.callback.as_ref())?;
        }
        if let Some(callback) = resource {
            visit.call(callback.as_ref())?;
        }
        if let Some(callback) = maps {
            visit.call(callback.as_ref())?;
        }
        if let Some(slot) = stream {
            visit.call(slot.callback.as_ref())?;
            visit.call(slot.event_loop.as_ref())?;
            visit.call(slot.context.as_ref())?;
        }
        self.sources.visit(visit)
    }

    /// Fence every Python callback so native services cannot start new
    /// application work while the Domain is leaving. Active stream sources
    /// stay registered until the native protocol hosts have stopped.
    pub(crate) fn fence(&self) {
        let participant = self.participant.write().take();
        let resource = self.resource.write().take();
        let maps = self.maps.write().take();
        let stream = self.stream.write().take();
        drop((participant, resource, maps, stream));
    }

    /// Drain Python-source finalizers after native Domain leave has dropped
    /// every stream pump, then release the remaining Python references.
    pub(crate) async fn finish_cleanup(&self) {
        self.sources.shutdown().await;
    }

    /// Synchronous cleanup for an unjoined builder, which cannot own active
    /// native stream pumps.
    pub(crate) fn clear(&self) {
        self.fence();
        self.sources.clear();
    }
}

fn sample_participant(
    py: Python<'_>,
    callback: &Py<PyAny>,
) -> PyResult<AuthenticatedParticipantInfo> {
    let value = callback.bind(py).call0()?;
    Ok(value.extract::<PyRef<PyParticipantInfo>>()?.inner.clone())
}

struct PythonParticipantInfo {
    slot: Arc<RwLock<Option<Arc<ParticipantSlot>>>>,
    fallback: RwLock<AuthenticatedParticipantInfo>,
}

impl ParticipantInfoProvider for PythonParticipantInfo {
    fn participant_info(&self) -> AuthenticatedParticipantInfo {
        let Some(slot) = self.slot.read().clone() else {
            return self.fallback.read().clone();
        };
        let sampled = Python::with_gil(|py| sample_participant(py, slot.callback.as_ref()));
        match sampled {
            Ok(info) => {
                *slot.last.write() = info.clone();
                *self.fallback.write() = info.clone();
                info
            }
            Err(error) => {
                tracing::warn!(%error, "Python participant info provider failed; using last valid sample");
                slot.last.read().clone()
            }
        }
    }
}

struct PythonResourceCatalog {
    callback: Arc<RwLock<Option<Arc<Py<PyAny>>>>>,
}

impl ResourceCatalogProvider for PythonResourceCatalog {
    fn snapshot(&self) -> Vec<auki_domain_rs::ResourceEntry> {
        let Some(callback) = self.callback.read().clone() else {
            return Vec::new();
        };
        Python::with_gil(|py| {
            callback
                .bind(py)
                .call0()
                .and_then(|value| resource_entries(&value))
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "Python Resource Catalog provider failed");
                    Vec::new()
                })
        })
    }
}

struct PythonMapCatalog {
    callback: Arc<RwLock<Option<Arc<Py<PyAny>>>>>,
}

impl MapCatalogProvider for PythonMapCatalog {
    fn map_catalog(&self) -> ResourcesResponseV4 {
        let Some(callback) = self.callback.read().clone() else {
            return ResourcesResponseV4 { resources: vec![] };
        };
        let resources = Python::with_gil(|py| {
            callback
                .bind(py)
                .call0()
                .and_then(|value| map_resources(&value))
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "Python Map Catalog provider failed");
                    Vec::new()
                })
        });
        ResourcesResponseV4 { resources }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::types::PyModule;

    fn info(name: &str) -> AuthenticatedParticipantInfo {
        AuthenticatedParticipantInfo {
            app: "test".into(),
            app_version: "1".into(),
            name: name.into(),
            session_id: "session".into(),
            session_clock_id: "clock".into(),
            session_clock_hash: "hash".into(),
            session_now_ns: 1,
            peer_id: auki_domain_rs::Identity::generate().peer_id(),
            app_instance: String::new(),
        }
    }

    #[test]
    fn participant_provider_samples_live_and_falls_back_after_clear() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let module = PyModule::from_code_bound(
                py,
                "def provider():\n    return current\n",
                "participant_provider_test.py",
                "participant_provider_test",
            )
            .unwrap();
            module
                .setattr(
                    "current",
                    Py::new(
                        py,
                        PyParticipantInfo {
                            inner: info("first"),
                        },
                    )
                    .unwrap(),
                )
                .unwrap();
            let slots = ProviderSlots::default();
            slots
                .set_participant(py, module.getattr("provider").unwrap().unbind())
                .unwrap();
            let provider = slots.participant_provider();
            assert_eq!(provider.participant_info().name, "first");

            module
                .setattr(
                    "current",
                    Py::new(
                        py,
                        PyParticipantInfo {
                            inner: info("second"),
                        },
                    )
                    .unwrap(),
                )
                .unwrap();
            assert_eq!(provider.participant_info().name, "second");
            slots.clear();
            assert_eq!(provider.participant_info().name, "second");
        });
    }
}
