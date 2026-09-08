//! Python client and provider-backed endpoint for Catalog v3 resources/v4 maps.

use auki_protocols::catalog::{CatalogClient, CatalogEndpoint, CatalogProvider, v3, v4};
use parking_lot::Mutex;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    pyclass::{PyTraverseError, PyVisit},
    types::{PyAny, PyModule},
};

use crate::{
    PyAukiPeer,
    cleanup::{CleanupResult, DetachedCleanup, wait_cleanup},
};

use super::support::{
    PythonCallback, enter_tokio_runtime, optional_callable, parse_peer_id, parse_python,
    parse_target, report_provider_error, requester_to_python, runtime_error, to_python,
};

const RESOURCE_VARIANT_NAMES: &str =
    "sensor_log, pose_log, time_transform_log, detection_log, or message_channel";

fn resources_request(variants: Option<Vec<String>>) -> PyResult<v3::ResourcesRequest> {
    let variants = variants
        .unwrap_or_default()
        .into_iter()
        .map(|variant| match variant.as_str() {
            "sensor_log" => Ok(v3::ResourceVariant::SensorLog),
            "pose_log" => Ok(v3::ResourceVariant::PoseLog),
            "time_transform_log" => Ok(v3::ResourceVariant::TimeTransformLog),
            "detection_log" => Ok(v3::ResourceVariant::DetectionLog),
            "message_channel" => Ok(v3::ResourceVariant::MessageChannel),
            _ => Err(PyValueError::new_err(format!(
                "Catalog resource variant must be one of {RESOURCE_VARIANT_NAMES}; got {variant:?}"
            ))),
        })
        .collect::<PyResult<Vec<_>>>()?;
    let request = v3::ResourcesRequest { variants };
    request.validate().map_err(|error| {
        PyValueError::new_err(format!("invalid Catalog resource filter: {error}"))
    })?;
    Ok(request)
}

/// Validate and normalize one Catalog v3 provider snapshot in Rust.
///
/// Preparing static snapshots when an application mounts its endpoint turns
/// schema mistakes into local startup errors instead of an empty, fail-closed
/// response observed only by a remote peer.
#[pyfunction]
fn prepare_catalog_resources(py: Python<'_>, response: Py<PyAny>) -> PyResult<PyObject> {
    let response: v3::ResourcesResponse =
        parse_python(py, response.bind(py), "Catalog resources snapshot")?;
    response.validate().map_err(|error| {
        PyValueError::new_err(format!("invalid Catalog resources snapshot: {error}"))
    })?;
    to_python(py, &response)
}

/// Outbound Catalog v3/v4 client backed by the portable Rust protocols.
#[pyclass(name = "AukiCatalogClient", frozen)]
#[derive(Clone)]
pub(crate) struct PyAukiCatalogClient {
    inner: CatalogClient,
}

impl PyAukiCatalogClient {
    fn from_inner(inner: CatalogClient) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyAukiCatalogClient {
    #[new]
    fn new(peer: &PyAukiPeer) -> Self {
        Self::from_inner(CatalogClient::new(peer.protocols()))
    }

    #[getter]
    fn resource_protocol(&self) -> &'static str {
        v3::ID
    }

    #[getter]
    fn maps_protocol(&self) -> &'static str {
        v4::ID
    }

    /// Fetch Catalog v3 resources using routes configured on the peer.
    /// An omitted or empty variant list requests every resource family.
    #[pyo3(signature = (remote_peer_id, variants=None))]
    fn fetch_resources<'py>(
        &self,
        py: Python<'py>,
        remote_peer_id: String,
        variants: Option<Vec<String>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let remote_peer_id = parse_peer_id(&remote_peer_id)?;
        let request = resources_request(variants)?;
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let response = client
                .fetch_resources(remote_peer_id, request)
                .await
                .map_err(|error| runtime_error("fetch Catalog resources", error))?;
            Python::with_gil(|py| to_python(py, &response))
        })
    }

    /// Fetch Catalog v3 resources through one exact advertised route.
    #[pyo3(signature = (remote_peer_id, route, variants=None))]
    fn fetch_resources_exact<'py>(
        &self,
        py: Python<'py>,
        remote_peer_id: String,
        route: String,
        variants: Option<Vec<String>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (remote_peer_id, route) = parse_target(&remote_peer_id, &route)?;
        let request = resources_request(variants)?;
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let response = client
                .fetch_resources_exact(remote_peer_id, route, request)
                .await
                .map_err(|error| runtime_error("fetch Catalog resources", error))?;
            Python::with_gil(|py| to_python(py, &response))
        })
    }

    /// Fetch Catalog v4 Map Logs using routes configured on the peer.
    fn fetch_maps<'py>(
        &self,
        py: Python<'py>,
        remote_peer_id: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let remote_peer_id = parse_peer_id(&remote_peer_id)?;
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let response = client
                .fetch_maps(remote_peer_id)
                .await
                .map_err(|error| runtime_error("fetch Catalog maps", error))?;
            Python::with_gil(|py| to_python(py, &response))
        })
    }

    /// Fetch Catalog v4 Map Logs through one exact advertised route.
    fn fetch_maps_exact<'py>(
        &self,
        py: Python<'py>,
        remote_peer_id: String,
        route: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (remote_peer_id, route) = parse_target(&remote_peer_id, &route)?;
        let client = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let response = client
                .fetch_maps_exact(remote_peer_id, route)
                .await
                .map_err(|error| runtime_error("fetch Catalog maps", error))?;
            Python::with_gil(|py| to_python(py, &response))
        })
    }
}

#[derive(Clone)]
struct PythonCatalogProvider {
    resources: Option<PythonCallback>,
    maps: Option<PythonCallback>,
}

impl CatalogProvider for PythonCatalogProvider {
    fn resources(
        &self,
        requester: &auki_sdk_rs::AuthenticatedPeer,
        request: &v3::ResourcesRequest,
    ) -> v3::ResourcesResponse {
        let Some(callback) = self.resources.as_ref() else {
            return v3::ResourcesResponse {
                resources: Vec::new(),
            };
        };
        Python::with_gil(|py| {
            let callback = callback.bind(py);
            let sampled = (|| {
                let requester = requester_to_python(py, requester)?;
                let request = to_python(py, request)?;
                let value = callback.call1((requester, request))?;
                if value.is_none() {
                    Ok(v3::ResourcesResponse {
                        resources: Vec::new(),
                    })
                } else {
                    parse_python(py, &value, "Catalog resources snapshot")
                }
            })();
            sampled.unwrap_or_else(|error| {
                report_provider_error(py, callback, error);
                v3::ResourcesResponse {
                    resources: Vec::new(),
                }
            })
        })
    }

    fn maps(&self, requester: &auki_sdk_rs::AuthenticatedPeer) -> v4::ResourcesResponse {
        let Some(callback) = self.maps.as_ref() else {
            return v4::ResourcesResponse {
                resources: Vec::new(),
            };
        };
        Python::with_gil(|py| {
            let callback = callback.bind(py);
            let sampled = (|| {
                let requester = requester_to_python(py, requester)?;
                let value = callback.call1((requester,))?;
                if value.is_none() {
                    Ok(v4::ResourcesResponse {
                        resources: Vec::new(),
                    })
                } else {
                    parse_python(py, &value, "Catalog maps snapshot")
                }
            })();
            sampled.unwrap_or_else(|error| {
                report_provider_error(py, callback, error);
                v4::ResourcesResponse {
                    resources: Vec::new(),
                }
            })
        })
    }
}

struct CatalogEndpointOwner {
    endpoint: Mutex<Option<CatalogEndpoint>>,
    callbacks: Mutex<Vec<PythonCallback>>,
    cleanup: DetachedCleanup,
}

impl CatalogEndpointOwner {
    fn new(
        endpoint: CatalogEndpoint,
        resources: Option<PythonCallback>,
        maps: Option<PythonCallback>,
    ) -> Self {
        Self {
            endpoint: Mutex::new(Some(endpoint)),
            callbacks: Mutex::new(resources.into_iter().chain(maps).collect()),
            cleanup: DetachedCleanup::default(),
        }
    }

    fn begin_close(&self) -> tokio::sync::watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            self.callbacks.lock().clear();
            let close = self.endpoint.lock().take().map(CatalogEndpoint::close);
            async move {
                match close {
                    Some(close) => close.await.map_err(|error| error.to_string()),
                    None => Ok(()),
                }
            }
        })
    }

    fn traverse(&self, visit: &PyVisit<'_>) -> Result<(), PyTraverseError> {
        for callback in self.callbacks.lock().iter() {
            visit.call(callback.as_ref())?;
        }
        Ok(())
    }
}

impl Drop for CatalogEndpointOwner {
    fn drop(&mut self) {
        self.callbacks.get_mut().clear();
        let Some(endpoint) = self.endpoint.get_mut().take() else {
            return;
        };
        pyo3_async_runtimes::tokio::get_runtime().spawn(async move {
            let _ = endpoint.close().await;
        });
    }
}

/// Mounted Catalog v3/v4 providers plus a cloneable outbound client.
#[pyclass(name = "AukiCatalogEndpoint")]
pub(crate) struct PyAukiCatalogEndpoint {
    owner: CatalogEndpointOwner,
    client: CatalogClient,
}

#[pymethods]
impl PyAukiCatalogEndpoint {
    /// Mount Catalog with synchronous snapshot callbacks.
    ///
    /// `resources_provider(requester, request)` returns one canonical v3
    /// response dict. `maps_provider(requester)` returns one canonical v4
    /// response dict. An omitted callback or `None` result is an empty catalog.
    #[staticmethod]
    #[pyo3(signature = (peer, resources_provider=None, maps_provider=None))]
    fn mount(
        py: Python<'_>,
        peer: &PyAukiPeer,
        resources_provider: Option<Py<PyAny>>,
        maps_provider: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let resources = optional_callable(py, resources_provider, "Catalog resources provider")?;
        let maps = optional_callable(py, maps_provider, "Catalog maps provider")?;
        let endpoint = enter_tokio_runtime(|| {
            CatalogEndpoint::mount(
                peer.protocols(),
                PythonCatalogProvider {
                    resources: resources.clone(),
                    maps: maps.clone(),
                },
            )
        })
        .map_err(|error| runtime_error("mount Catalog", error))?;
        let client = endpoint.client();
        Ok(Self {
            owner: CatalogEndpointOwner::new(endpoint, resources, maps),
            client,
        })
    }

    #[getter]
    fn resource_protocol(&self) -> &'static str {
        v3::ID
    }

    #[getter]
    fn maps_protocol(&self) -> &'static str {
        v4::ID
    }

    #[getter]
    fn client(&self) -> PyAukiCatalogClient {
        PyAukiCatalogClient::from_inner(self.client.clone())
    }

    /// Stop accepting Catalog requests behind one detached, replayable barrier.
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let cleanup = self.owner.begin_close();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            wait_cleanup(cleanup)
                .await
                .map_err(|error| runtime_error("close Catalog", error))
        })
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        self.owner.traverse(&visit)
    }

    fn __clear__(&mut self) {
        self.owner.begin_close();
    }
}

pub(super) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(prepare_catalog_resources, module)?)?;
    module.add_class::<PyAukiCatalogClient>()?;
    module.add_class::<PyAukiCatalogEndpoint>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use auki_sdk_rs::Identity;

    use super::super::support::{requester, require_callable};
    use super::*;

    #[test]
    fn resource_filter_uses_only_exact_canonical_names() {
        let request =
            resources_request(Some(vec!["sensor_log".into(), "message_channel".into()])).unwrap();
        assert_eq!(
            request.variants,
            vec![
                v3::ResourceVariant::SensorLog,
                v3::ResourceVariant::MessageChannel,
            ]
        );
        assert!(resources_request(None).unwrap().variants.is_empty());
        assert!(resources_request(Some(vec!["SensorLog".into()])).is_err());
        assert!(resources_request(Some(vec!["sensor_log".into(), "sensor_log".into()])).is_err());
    }

    #[test]
    fn prepared_catalog_snapshots_are_validated_and_normalized() {
        Python::with_gil(|py| {
            let json = py.import_bound("json").unwrap();
            let owner = Identity::generate().peer_id().to_string();
            let snapshot = json
                .call_method1(
                    "loads",
                    (format!(
                        r#"{{
                            "resources": [{{
                                "variant": "message_channel",
                                "owner_peer_id": "{owner}",
                                "resource_id": "camera/replies",
                                "clock": {{
                                    "peer_id": "{owner}",
                                    "id": "camera/utc",
                                    "hash": "{}"
                                }}
                            }}]
                        }}"#,
                        "a".repeat(32)
                    ),),
                )
                .unwrap()
                .unbind();

            let prepared = prepare_catalog_resources(py, snapshot).unwrap();
            assert_eq!(
                prepared
                    .bind(py)
                    .get_item("resources")
                    .unwrap()
                    .len()
                    .unwrap(),
                1
            );

            let invalid = json
                .call_method1(
                    "loads",
                    (r#"{"resources":[{"variant":"message_channel","owner_peer_id":"bad","resource_id":""}]}"#,),
                )
                .unwrap()
                .unbind();
            assert!(prepare_catalog_resources(py, invalid).is_err());
        });
    }

    #[test]
    fn synchronous_catalog_callbacks_receive_canonical_records() {
        Python::with_gil(|py| {
            let module = PyModule::from_code_bound(
                py,
                r#"
def resources(requester, request):
    assert requester["peer_type"] == "native_app"
    assert request == {"variants": ["sensor_log"]}
    return {"resources": []}

def maps(requester):
    assert "verified_until" in requester
    return {"resources": []}
"#,
                "catalog_provider_test.py",
                "catalog_provider_test",
            )
            .unwrap();
            let provider = PythonCatalogProvider {
                resources: Some(
                    require_callable(
                        py,
                        module.getattr("resources").unwrap().unbind(),
                        "Catalog resources provider",
                    )
                    .unwrap(),
                ),
                maps: Some(
                    require_callable(
                        py,
                        module.getattr("maps").unwrap().unbind(),
                        "Catalog maps provider",
                    )
                    .unwrap(),
                ),
            };
            let requester = requester(Identity::generate().peer_id());
            let resources = provider.resources(
                &requester,
                &v3::ResourcesRequest {
                    variants: vec![v3::ResourceVariant::SensorLog],
                },
            );
            assert!(resources.resources.is_empty());
            assert!(provider.maps(&requester).resources.is_empty());
        });
    }

    #[test]
    fn omitted_callbacks_serve_empty_snapshots() {
        let provider = PythonCatalogProvider {
            resources: None,
            maps: None,
        };
        let requester = requester(Identity::generate().peer_id());
        assert!(
            provider
                .resources(&requester, &v3::ResourcesRequest::all())
                .resources
                .is_empty()
        );
        assert!(provider.maps(&requester).resources.is_empty());
    }
}
