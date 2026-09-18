//! Domain data bindings. Owned tasks survive Python future cancellation long
//! enough to observe the cancellation token and abort multipart sessions.
use auki_sdk_rs::{
    AukiDomains, DataError, DataListQuery, DataWrite, DomainDataClient, DomainListQuery, PortalId,
    TransferOptions,
};
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict, PyModule},
};
use serde::Serialize;
use std::future::Future;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// PyO3 0.22 expands a compatibility cfg for its retired gil-refs feature.
#[allow(unexpected_cfgs)]
mod exception {
    pyo3::create_exception!(auki_sdk, DomainDataError, pyo3::exceptions::PyRuntimeError);
}
use exception::DomainDataError;
fn error(error: DataError) -> PyErr {
    Python::with_gil(|py| {
        let result = DomainDataError::new_err(error.to_string());
        let _ = result.value_bound(py).setattr("status", error.status());
        if let Some(code) = auth_code(&error) {
            let _ = result.value_bound(py).setattr("code", code);
        }
        let kind = match error {
            DataError::Cancelled => "cancelled",
            DataError::Closed => "closed",
            DataError::TimedOut => "timeout",
            DataError::Auth(_) => "auth",
            DataError::HttpStatus { .. } => "http",
            DataError::InvalidInput(_) => "input",
            DataError::InvalidResponse(_) => "response",
            DataError::TooLarge { .. } => "limit",
            DataError::Callback => "callback",
            DataError::Cleanup { .. } => "cleanup",
            DataError::Transport => "transport",
        };
        let _ = result.value_bound(py).setattr("kind", kind);
        result
    })
}
fn auth_code(error: &DataError) -> Option<&'static str> {
    match error {
        DataError::Auth(error) => Some(crate::zitadel::auth_code(error.kind())),
        DataError::Cleanup { operation, .. } => auth_code(operation),
        _ => None,
    }
}
pub(crate) fn id(value: &str) -> PyResult<Uuid> {
    Uuid::parse_str(value).map_err(|_| PyValueError::new_err("expected UUID"))
}
fn json(value: &impl Serialize) -> PyResult<PyObject> {
    let value = serde_json::to_string(value)
        .map_err(|_| PyRuntimeError::new_err("cannot convert metadata"))?;
    Python::with_gil(|py| {
        Ok(PyModule::import_bound(py, "json")?
            .getattr("loads")?
            .call1((value,))?
            .unbind())
    })
}
fn none() -> PyObject {
    Python::with_gil(|py| py.None())
}
fn run<'py, F, Fut>(py: Python<'py>, operation: F) -> PyResult<Bound<'py, PyAny>>
where
    F: FnOnce(CancellationToken) -> Fut,
    Fut: Future<Output = PyResult<PyObject>> + Send + 'static,
{
    let token = CancellationToken::new();
    let guard = token.clone().drop_guard();
    let task = pyo3_async_runtimes::tokio::get_runtime().spawn(operation(token));
    crate::async_completion::future_into_py(py, async move {
        let _guard = guard;
        task.await
            .map_err(|_| PyRuntimeError::new_err("Domain operation stopped unexpectedly"))?
    })
}

#[pyclass(name = "AukiDomains")]
pub(crate) struct PyDomains {
    pub(crate) inner: AukiDomains,
}
#[pymethods]
impl PyDomains {
    #[pyo3(signature = (portal_id, *, organization="own", limit=50, cursor=None))]
    fn for_portal_page<'py>(
        &self,
        py: Python<'py>,
        portal_id: &str,
        organization: &str,
        limit: usize,
        cursor: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let portal = PortalId::parse(portal_id).map_err(|e| error(e.into()))?;
        let organization = organization.to_owned();
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            json(
                &inner
                    .for_portal_page(&portal, &organization, limit, cursor.as_deref(), &cancel)
                    .await
                    .map_err(error)?,
            )
        })
    }
    #[pyo3(signature = (domain_id, *, limit=50, cursor=None))]
    fn portals_page<'py>(
        &self,
        py: Python<'py>,
        domain_id: &str,
        limit: usize,
        cursor: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let domain = id(domain_id)?;
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            json(
                &inner
                    .portals_page(domain, limit, cursor.as_deref(), &cancel)
                    .await
                    .map_err(error)?,
            )
        })
    }

    #[pyo3(signature = (*, organization="own", limit=50, offset=0, domain_server_id=None))]
    fn list<'py>(
        &self,
        py: Python<'py>,
        organization: &str,
        limit: u32,
        offset: u32,
        domain_server_id: Option<&str>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let query = DomainListQuery {
            organization: organization.into(),
            limit,
            offset,
            domain_server_id: domain_server_id.map(id).transpose()?,
        };
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            json(
                &inner
                    .list_with_cancellation(&query, &cancel)
                    .await
                    .map_err(error)?,
            )
        })
    }
    #[pyo3(signature = (portal_id, *, organization="own"))]
    fn for_portal<'py>(
        &self,
        py: Python<'py>,
        portal_id: &str,
        organization: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let portal = PortalId::parse(portal_id).map_err(|e| error(e.into()))?;
        let organization = organization.to_owned();
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            json(
                &inner
                    .for_portal(&portal, &organization, &cancel)
                    .await
                    .map_err(error)?,
            )
        })
    }
    fn portals<'py>(&self, py: Python<'py>, domain_id: &str) -> PyResult<Bound<'py, PyAny>> {
        let domain = id(domain_id)?;
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            json(&inner.portals(domain, &cancel).await.map_err(error)?)
        })
    }
    fn portal<'py>(
        &self,
        py: Python<'py>,
        domain_id: &str,
        portal_id: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let domain = id(domain_id)?;
        let portal = PortalId::parse(portal_id).map_err(|e| error(e.into()))?;
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            json(
                &inner
                    .portal(domain, &portal, &cancel)
                    .await
                    .map_err(error)?,
            )
        })
    }
}

struct Target {
    id: Option<Uuid>,
    name: Option<String>,
    data_type: Option<String>,
}
impl Target {
    fn new(
        data_id: Option<&str>,
        name: Option<String>,
        data_type: Option<String>,
    ) -> PyResult<Self> {
        let result = Self {
            id: data_id.map(id).transpose()?,
            name,
            data_type,
        };
        result.as_write()?;
        Ok(result)
    }
    fn as_write(&self) -> PyResult<DataWrite<'_>> {
        match (&self.id, &self.name, &self.data_type) {
            (Some(id), None, None) => Ok(DataWrite::ById(*id)),
            (None, Some(name), Some(data_type)) => Ok(DataWrite::Named { name, data_type }),
            _ => Err(PyValueError::new_err(
                "provide data_id or both name and data_type",
            )),
        }
    }
}
#[pyclass(name = "AukiDomainData")]
pub(crate) struct PyData {
    pub(crate) inner: DomainDataClient,
}
#[pymethods]
impl PyData {
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        run(py, |_| async move {
            inner.close().await;
            Ok(none())
        })
    }
    #[pyo3(signature = (*, ids=None, name=None, data_type=None))]
    fn list<'py>(
        &self,
        py: Python<'py>,
        ids: Option<Vec<String>>,
        name: Option<String>,
        data_type: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let query = DataListQuery {
            ids: ids
                .unwrap_or_default()
                .iter()
                .map(|value| id(value))
                .collect::<PyResult<_>>()?,
            name,
            data_type,
        };
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            json(
                &inner
                    .list_with_cancellation(&query, &cancel)
                    .await
                    .map_err(error)?,
            )
        })
    }
    fn get<'py>(&self, py: Python<'py>, data_id: &str) -> PyResult<Bound<'py, PyAny>> {
        let id = id(data_id)?;
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            json(
                &inner
                    .get_with_cancellation(id, &cancel)
                    .await
                    .map_err(error)?,
            )
        })
    }
    fn read<'py>(&self, py: Python<'py>, data_id: &str) -> PyResult<Bound<'py, PyAny>> {
        let id = id(data_id)?;
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            let bytes = inner
                .read_with_cancellation(id, &cancel)
                .await
                .map_err(error)?;
            Python::with_gil(|py| Ok(PyBytes::new_bound(py, &bytes).unbind().into_any()))
        })
    }
    #[pyo3(signature = (data, *, data_id=None, name=None, data_type=None))]
    fn write<'py>(
        &self,
        py: Python<'py>,
        data: &Bound<'_, PyBytes>,
        data_id: Option<&str>,
        name: Option<String>,
        data_type: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if data.as_bytes().len() > 8 * 1024 * 1024 {
            return Err(error(DataError::TooLarge {
                maximum: 8 * 1024 * 1024,
            }));
        }
        let bytes = data.as_bytes().to_vec();
        let target = Target::new(data_id, name, data_type)?;
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            json(
                &inner
                    .write_with_cancellation(target.as_write()?, &bytes, &cancel)
                    .await
                    .map_err(error)?,
            )
        })
    }
    fn delete<'py>(&self, py: Python<'py>, data_id: &str) -> PyResult<Bound<'py, PyAny>> {
        let id = id(data_id)?;
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            inner
                .delete_with_cancellation(id, &cancel)
                .await
                .map_err(error)?;
            Ok(none())
        })
    }
    fn poses<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            json(&inner.poses(&cancel).await.map_err(error)?)
        })
    }
    fn pose<'py>(&self, py: Python<'py>, portal_id: &str) -> PyResult<Bound<'py, PyAny>> {
        let portal = PortalId::parse(portal_id).map_err(|e| error(e.into()))?;
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            json(&inner.pose(&portal, &cancel).await.map_err(error)?)
        })
    }
    /// Await sink(bytes) for each chunk, preserving backpressure.
    #[pyo3(signature = (data_id, sink, *, max_bytes=8589934592, max_chunk_bytes=16777216))]
    fn read_to<'py>(
        &self,
        py: Python<'py>,
        data_id: &str,
        sink: PyObject,
        max_bytes: u64,
        max_chunk_bytes: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        let id = id(data_id)?;
        let inner = self.inner.clone();
        let locals = pyo3_async_runtimes::tokio::get_current_locals(py)?;
        run(py, |cancel| async move {
            let size = inner
                .read_to(
                    id,
                    TransferOptions {
                        max_bytes,
                        max_chunk_bytes,
                    },
                    &cancel,
                    |bytes| {
                        let future = Python::with_gil(|py| {
                            callback(
                                &sink,
                                PyBytes::new_bound(py, &bytes).unbind().into_any(),
                                &locals,
                            )
                        });
                        async move {
                            future
                                .map_err(|_| DataError::Callback)?
                                .await
                                .map_err(|_| DataError::Callback)?;
                            Ok(())
                        }
                    },
                )
                .await
                .map_err(error)?;
            Python::with_gil(|py| Ok(size.into_py(py)))
        })
    }
    /// Await source(maximum_bytes); return bytes, with b"" indicating EOF.
    /// Named multipart completion can replace existing names.
    #[pyo3(signature = (size, source, *, data_id=None, name=None, data_type=None, max_bytes=8589934592, max_chunk_bytes=16777216))]
    #[allow(clippy::too_many_arguments)]
    fn write_stream<'py>(
        &self,
        py: Python<'py>,
        size: u64,
        source: PyObject,
        data_id: Option<&str>,
        name: Option<String>,
        data_type: Option<String>,
        max_bytes: u64,
        max_chunk_bytes: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        let target = Target::new(data_id, name, data_type)?;
        let inner = self.inner.clone();
        let locals = pyo3_async_runtimes::tokio::get_current_locals(py)?;
        run(py, |cancel| async move {
            let metadata = inner
                .write_stream(
                    target.as_write()?,
                    size,
                    TransferOptions {
                        max_bytes,
                        max_chunk_bytes,
                    },
                    &cancel,
                    |maximum| {
                        let future =
                            Python::with_gil(|py| callback(&source, maximum.into_py(py), &locals));
                        async move {
                            let bytes = future
                                .map_err(|_| DataError::Callback)?
                                .await
                                .map_err(|_| DataError::Callback)?;
                            Python::with_gil(|py| {
                                let bytes = bytes
                                    .bind(py)
                                    .downcast::<PyBytes>()
                                    .map_err(|_| DataError::Callback)?
                                    .as_bytes();
                                if bytes.len() > maximum {
                                    return Err(DataError::InvalidInput(
                                        "source exceeded requested chunk size",
                                    ));
                                }
                                Ok(bytes.to_vec())
                            })
                        }
                    },
                )
                .await
                .map_err(error)?;
            json(&metadata)
        })
    }
}
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyDomains>()?;
    module.add_class::<PyData>()?;
    module.add(
        "DomainDataError",
        module.py().get_type_bound::<DomainDataError>(),
    )?;
    Ok(())
}

// run_coroutine_threadsafe owns callback scheduling and gives cancellation a
// thread-safe handle. Dropping the Rust callback future cancels the Python task.
struct CancelCallback(PyObject);
impl Drop for CancelCallback {
    fn drop(&mut self) {
        Python::with_gil(|py| {
            let _ = self.0.bind(py).call_method0("cancel");
        });
    }
}
fn callback(
    function: &PyObject,
    argument: PyObject,
    locals: &pyo3_async_runtimes::TaskLocals,
) -> PyResult<impl Future<Output = PyResult<PyObject>> + Send + use<>> {
    Python::with_gil(|py| {
        static INVOKE: pyo3::sync::GILOnceCell<PyObject> = pyo3::sync::GILOnceCell::new();
        let invoke = INVOKE.get_or_try_init(py, || -> PyResult<PyObject> {
            Ok(PyModule::from_code_bound(
                py,
                "async def invoke(callback, argument):\n    return await callback(argument)\n",
                "_auki_data_callback.py",
                "_auki_data_callback",
            )?
            .getattr("invoke")?
            .unbind())
        })?;
        let coroutine = invoke.bind(py).call1((function.bind(py), argument))?;
        let asyncio = PyModule::import_bound(py, "asyncio")?;
        let task = asyncio
            .getattr("run_coroutine_threadsafe")?
            .call1((coroutine, locals.event_loop(py)))?;
        let kwargs = PyDict::new_bound(py);
        kwargs.set_item("loop", locals.event_loop(py))?;
        let wrapped = asyncio
            .getattr("wrap_future")?
            .call((task.clone(),), Some(&kwargs))?;
        let future = pyo3_async_runtimes::into_future_with_locals(locals, wrapped)?;
        let guard = CancelCallback(task.unbind());
        Ok(async move {
            let _guard = guard;
            future.await
        })
    })
}
