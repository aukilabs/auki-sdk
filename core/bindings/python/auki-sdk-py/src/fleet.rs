use auki_sdk_rs::{ComputePoolQuery, DomainFleetClient, FleetError, FleetQuery, JobMode};
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::PyModule,
};
use serde::Serialize;
use std::future::Future;
use tokio_util::sync::CancellationToken;

#[allow(unexpected_cfgs)]
mod exception {
    pyo3::create_exception!(auki_sdk, AukiFleetError, pyo3::exceptions::PyRuntimeError);
}
use exception::AukiFleetError;

pub(crate) fn error(error: FleetError) -> PyErr {
    Python::with_gil(|py| {
        let result = AukiFleetError::new_err(error.to_string());
        let value = result.value_bound(py);
        let _ = value.setattr("kind", error.kind());
        let _ = value.setattr("code", error.code());
        let _ = value.setattr("status", error.http_status());
        result
    })
}

fn json(value: &impl Serialize) -> PyResult<PyObject> {
    let encoded = serde_json::to_string(value)
        .map_err(|_| PyRuntimeError::new_err("cannot encode fleet snapshot"))?;
    Python::with_gil(|py| {
        Ok(PyModule::import_bound(py, "json")?
            .getattr("loads")?
            .call1((encoded,))?
            .unbind())
    })
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
            .map_err(|_| PyRuntimeError::new_err("fleet operation stopped unexpectedly"))?
    })
}

#[pyclass(name = "AukiFleet")]
pub(crate) struct PyFleet {
    pub(crate) inner: DomainFleetClient,
}

#[pymethods]
impl PyFleet {
    #[getter]
    fn domain_id(&self) -> String {
        self.inner.domain_id().to_string()
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        run(py, |_| async move {
            inner.close().await;
            Ok(Python::with_gil(|py| py.None()))
        })
    }

    #[pyo3(signature=(*,capabilities=None,match_all_capabilities=false))]
    fn list<'py>(
        &self,
        py: Python<'py>,
        capabilities: Option<Vec<String>>,
        match_all_capabilities: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let query = FleetQuery {
            capabilities: capabilities.unwrap_or_default(),
            match_all_capabilities,
        };
        run(py, |cancel| async move {
            json(
                &inner
                    .list_with_cancellation(&query, &cancel)
                    .await
                    .map_err(error)?,
            )
        })
    }

    #[pyo3(signature=(*,mode,capabilities=None,match_all_capabilities=false))]
    fn compute_pool<'py>(
        &self,
        py: Python<'py>,
        mode: &str,
        capabilities: Option<Vec<String>>,
        match_all_capabilities: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mode = match mode {
            "public" => JobMode::Public,
            "dedicated" => JobMode::Dedicated,
            _ => return Err(PyValueError::new_err("mode must be public or dedicated")),
        };
        let inner = self.inner.clone();
        let query = ComputePoolQuery {
            mode,
            capabilities: capabilities.unwrap_or_default(),
            match_all_capabilities,
        };
        run(py, |cancel| async move {
            json(
                &inner
                    .compute_pool_with_cancellation(&query, &cancel)
                    .await
                    .map_err(error)?,
            )
        })
    }
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add(
        "AukiFleetError",
        module.py().get_type_bound::<AukiFleetError>(),
    )?;
    module.add_class::<PyFleet>()
}
