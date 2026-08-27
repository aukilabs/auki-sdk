use std::{ffi::CString, sync::Arc};

use auki_session_rs::{
    PY_DOMAIN_PEER_CAPSULE_ABI_NAME as PEER_CAPSULE_NAME,
    PY_DOMAIN_SESSION_CAPSULE_ABI_NAME as SESSION_CAPSULE_NAME, Peer, Session,
};
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyCapsule};

pub(crate) fn peer(value: &Bound<'_, PyAny>) -> PyResult<Arc<Peer>> {
    extract(value, PEER_CAPSULE_NAME, "Peer")
}

pub(crate) fn session(value: &Bound<'_, PyAny>) -> PyResult<Arc<Session>> {
    extract(value, SESSION_CAPSULE_NAME, "Session")
}

fn extract<T>(value: &Bound<'_, PyAny>, expected_name: &str, kind: &str) -> PyResult<Arc<T>>
where
    T: Send + Sync + 'static,
{
    let object = value.call_method0("_domain_handle").map_err(|error| {
        PyTypeError::new_err(format!(
            "{kind} must be an auki_session object from the same SDK release: {error}"
        ))
    })?;
    let capsule = object.downcast::<PyCapsule>().map_err(|error| {
        PyTypeError::new_err(format!(
            "{kind} returned a non-capsule Domain handle: {error}"
        ))
    })?;
    let expected = CString::new(expected_name).expect("static capsule name has no nul");
    match capsule.name()? {
        Some(actual) if actual == expected.as_c_str() => {}
        Some(actual) => {
            return Err(PyTypeError::new_err(format!(
                "{kind} Domain handle has ABI name {actual:?}; expected {expected_name:?}"
            )));
        }
        None => {
            return Err(PyTypeError::new_err(format!(
                "{kind} Domain handle has no ABI name"
            )));
        }
    }

    // SAFETY: the exact versioned capsule name pins this paired-wheel ABI to
    // `Arc<T>`. A mismatched SDK release fails before the payload is read.
    let handle = unsafe { capsule.reference::<Arc<T>>() };
    Ok(Arc::clone(handle))
}
