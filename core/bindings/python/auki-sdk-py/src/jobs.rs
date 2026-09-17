//! DMS job bindings share the authenticated session and do not start a peer.
use auki_sdk_rs::{DomainJobsClient, JobListQuery, JobSpec, JobStatus, JobsError};
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::PyModule,
};
use serde::{Serialize, de::DeserializeOwned};
use std::future::Future;
use tokio_util::sync::CancellationToken;

#[allow(unexpected_cfgs)]
mod exception {
    pyo3::create_exception!(auki_sdk, AukiJobsError, pyo3::exceptions::PyRuntimeError);
}
use exception::AukiJobsError;

pub(crate) fn error(error: JobsError) -> PyErr {
    Python::with_gil(|py| {
        let result = AukiJobsError::new_err(error.to_string());
        let code = error.code();
        let _ = result.value_bound(py).setattr("kind", kind(&error));
        let _ = result.value_bound(py).setattr("code", code);
        let _ = result
            .value_bound(py)
            .setattr("status", error.http_status());
        if let JobsError::SubmissionUncertain { source } = &error {
            let _ = result.value_bound(py).setattr("source", source.code());
        }
        result
    })
}

fn kind(error: &JobsError) -> &'static str {
    match error {
        JobsError::Auth(_) => "auth",
        JobsError::InvalidInput(_) => "input",
        JobsError::InvalidResponse(_) => "response",
        JobsError::HttpStatus { .. } => "http",
        JobsError::Transport => "transport",
        JobsError::TimedOut => "timeout",
        JobsError::Cancelled => "cancelled",
        JobsError::Closed => "closed",
        JobsError::TooLarge { .. } => "limit",
        JobsError::SubmissionUncertain { .. } => "submission_uncertain",
    }
}

fn json(value: &impl Serialize) -> PyResult<PyObject> {
    let value = serde_json::to_string(value)
        .map_err(|_| PyRuntimeError::new_err("cannot convert job response"))?;
    Python::with_gil(|py| {
        Ok(PyModule::import_bound(py, "json")?
            .getattr("loads")?
            .call1((value,))?
            .unbind())
    })
}

fn parse<T: DeserializeOwned>(value: &Bound<'_, PyAny>, description: &str) -> PyResult<T> {
    let encoded: String = PyModule::import_bound(value.py(), "json")?
        .getattr("dumps")?
        .call1((value,))
        .map_err(|_| PyValueError::new_err(format!("invalid {description}")))?
        .extract()
        .map_err(|_| PyValueError::new_err(format!("invalid {description}")))?;
    serde_json::from_str(&encoded)
        .map_err(|_| PyValueError::new_err(format!("invalid {description}")))
}

fn parse_spec(value: &Bound<'_, PyAny>) -> PyResult<JobSpec> {
    let spec: JobSpec = parse(value, "job specification")?;
    if !spec.meta.is_object() || spec.tasks.iter().any(|task| !task.meta.is_object()) {
        return Err(PyValueError::new_err(
            "job and task meta must be JSON objects",
        ));
    }
    Ok(spec)
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
            .map_err(|_| PyRuntimeError::new_err("DMS job operation stopped unexpectedly"))?
    })
}

#[pyclass(name = "AukiDmsJobs")]
pub(crate) struct PyJobs {
    pub(crate) inner: DomainJobsClient,
}

#[pymethods]
impl PyJobs {
    #[getter]
    fn domain_id(&self) -> String {
        self.inner.domain_id().to_string()
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        run(py, |_| async move {
            inner.close().await;
            Ok(none())
        })
    }

    fn estimate<'py>(
        &self,
        py: Python<'py>,
        spec: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let spec = parse_spec(spec)?;
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            json(
                &inner
                    .estimate_with_cancellation(&spec, &cancel)
                    .await
                    .map_err(error)?,
            )
        })
    }

    fn submit<'py>(&self, py: Python<'py>, spec: &Bound<'_, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let spec = parse_spec(spec)?;
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            let id = inner
                .submit_with_cancellation(&spec, &cancel)
                .await
                .map_err(error)?;
            Python::with_gil(|py| Ok(id.to_string().into_py(py)))
        })
    }

    #[pyo3(signature = (*, limit=50, cursor=None, status=None, capabilities=None, match_all_capabilities=false))]
    fn list<'py>(
        &self,
        py: Python<'py>,
        limit: u32,
        cursor: Option<String>,
        status: Option<String>,
        capabilities: Option<Vec<String>>,
        match_all_capabilities: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let status = status
            .map(|value| serde_json::from_value::<JobStatus>(value.into()))
            .transpose()
            .map_err(|_| PyValueError::new_err("invalid job status"))?;
        let query = JobListQuery {
            limit,
            cursor,
            status,
            capabilities: capabilities.unwrap_or_default(),
            match_all_capabilities,
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

    fn get<'py>(&self, py: Python<'py>, job_id: &str) -> PyResult<Bound<'py, PyAny>> {
        let job_id = crate::data::id(job_id)?;
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            json(
                &inner
                    .get_with_cancellation(job_id, &cancel)
                    .await
                    .map_err(error)?,
            )
        })
    }

    fn cancel<'py>(&self, py: Python<'py>, job_id: &str) -> PyResult<Bound<'py, PyAny>> {
        let job_id = crate::data::id(job_id)?;
        let inner = self.inner.clone();
        run(py, |cancel| async move {
            json(
                &inner
                    .cancel_with_cancellation(job_id, &cancel)
                    .await
                    .map_err(error)?,
            )
        })
    }
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add(
        "AukiJobsError",
        module.py().get_type_bound::<AukiJobsError>(),
    )?;
    module.add_class::<PyJobs>()
}
