//! Python values and awaited storage adapter for imported ZITADEL sessions.

use std::sync::Arc;

use auki_sdk_rs::{AuthError, AuthFailureKind, ZitadelSessionCredentials, ZitadelSessionStore};
use chrono::{DateTime, SecondsFormat, Utc};
use pyo3::{exceptions::PyRuntimeError, prelude::*};
use pyo3_async_runtimes::TaskLocals;

use crate::python_task::CancelablePythonAwaitable;

fn configuration_error() -> PyErr {
    runtime_error("invalid ZITADEL session configuration", "configuration")
}

fn runtime_error(message: &'static str, code: &'static str) -> PyErr {
    let result = PyRuntimeError::new_err(message);
    Python::with_gil(|py| {
        let _ = result.value_bound(py).setattr("code", code);
    });
    result
}

/// Opaque credential snapshot. Token values require explicit expose methods.
#[pyclass(name = "ZitadelSessionCredentials", frozen)]
pub(crate) struct PyZitadelSessionCredentials {
    credentials: ZitadelSessionCredentials,
}

impl PyZitadelSessionCredentials {
    pub(crate) fn copy_from(credentials: &ZitadelSessionCredentials) -> Self {
        Self {
            credentials: ZitadelSessionCredentials::new(
                credentials.access_token().expose_secret(),
                credentials.refresh_token().expose_secret(),
                credentials.client_id(),
                credentials.issuer().clone(),
                credentials.access_token_expires_at(),
            )
            .expect("copy of validated ZITADEL credentials"),
        }
    }

    pub(crate) fn copy_credentials(&self) -> ZitadelSessionCredentials {
        Self::copy_from(&self.credentials).credentials
    }
}

#[pymethods]
impl PyZitadelSessionCredentials {
    #[new]
    #[pyo3(signature = (access_token, refresh_token, client_id, issuer, access_token_expires_at=None))]
    fn new(
        access_token: String,
        refresh_token: String,
        client_id: String,
        issuer: String,
        access_token_expires_at: Option<String>,
    ) -> PyResult<Self> {
        let expiry = access_token_expires_at
            .map(|value| {
                DateTime::parse_from_rfc3339(&value)
                    .map(|value| value.with_timezone(&Utc))
                    .map_err(|_| configuration_error())
            })
            .transpose()?;
        let credentials = ZitadelSessionCredentials::new(
            access_token,
            refresh_token,
            client_id,
            issuer.parse().map_err(|_| configuration_error())?,
            expiry,
        )
        .map_err(|_| configuration_error())?;
        Ok(Self { credentials })
    }

    fn expose_access_token(&self) -> String {
        self.credentials.access_token().expose_secret().into()
    }

    fn expose_refresh_token(&self) -> String {
        self.credentials.refresh_token().expose_secret().into()
    }

    #[getter]
    fn client_id(&self) -> String {
        self.credentials.client_id().into()
    }

    #[getter]
    fn issuer(&self) -> String {
        self.credentials.issuer().to_string()
    }

    #[getter]
    fn access_token_expires_at(&self) -> Option<String> {
        self.credentials
            .access_token_expires_at()
            .map(|value| value.to_rfc3339_opts(SecondsFormat::AutoSi, true))
    }

    fn __repr__(&self) -> &'static str {
        "ZitadelSessionCredentials([redacted])"
    }

    fn __str__(&self) -> &'static str {
        "ZitadelSessionCredentials([redacted])"
    }
}

/// Calls one Python async store on the event loop captured during import.
pub(crate) struct PythonZitadelStore {
    callback: Arc<Py<PyAny>>,
    locals: Arc<TaskLocals>,
}

impl PythonZitadelStore {
    pub(crate) fn new(py: Python<'_>, callback: PyObject) -> PyResult<Self> {
        if !callback.bind(py).is_callable() {
            return Err(runtime_error(
                "ZITADEL session store must be an async callable",
                "configuration",
            ));
        }
        let locals = pyo3_async_runtimes::tokio::get_current_locals(py)?.copy_context(py)?;
        Ok(Self {
            callback: Arc::new(callback),
            locals: Arc::new(locals),
        })
    }
}

#[async_trait::async_trait]
impl ZitadelSessionStore for PythonZitadelStore {
    async fn save(&self, credentials: &ZitadelSessionCredentials) -> Result<(), AuthError> {
        let scheduled = Python::with_gil(|py| {
            let snapshot = Py::new(py, PyZitadelSessionCredentials::copy_from(credentials))?;
            let awaitable = self.callback.bind(py).call1((snapshot,))?;
            CancelablePythonAwaitable::schedule(py, &self.locals, awaitable)
        })
        .map_err(|_| AuthError::Persistence)?;
        scheduled.await.map_err(|_| AuthError::Persistence)?;
        Ok(())
    }
}

pub(crate) fn auth_error(error: impl Into<AuthError>) -> PyErr {
    let error = error.into();
    let kind = error.kind();
    let message = match kind {
        AuthFailureKind::AuthenticationRequired => "sign in again",
        AuthFailureKind::Configuration => "invalid authentication configuration",
        AuthFailureKind::AuthorizationDenied => "Domain access is denied",
        AuthFailureKind::Persistence => {
            "could not persist replacement credentials; retry using this session"
        }
        AuthFailureKind::Transient => "authentication is temporarily unavailable",
        AuthFailureKind::Cancelled => "authentication operation was cancelled",
        AuthFailureKind::Closed => "the session is closed",
    };
    runtime_error(message, auth_code(kind))
}

pub(crate) fn auth_code(kind: AuthFailureKind) -> &'static str {
    match kind {
        AuthFailureKind::AuthenticationRequired => "authentication_required",
        AuthFailureKind::Configuration => "configuration",
        AuthFailureKind::AuthorizationDenied => "authorization_denied",
        AuthFailureKind::Persistence => "persistence",
        AuthFailureKind::Transient => "transient",
        AuthFailureKind::Cancelled => "cancelled",
        AuthFailureKind::Closed => "closed",
    }
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyZitadelSessionCredentials>()
}
