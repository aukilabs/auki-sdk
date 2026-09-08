//! Small implementation helpers shared by protocol endpoints.

use std::{future::Future, time::Duration};

#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

use futures::{FutureExt, pin_mut};
use futures_timer::Delay;

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) type Shared<T> = Arc<T>;
#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub(crate) type Shared<T> = Rc<T>;

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn share<T>(value: T) -> Shared<T> {
    Arc::new(value)
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub(crate) fn share<T>(value: T) -> Shared<T> {
    Rc::new(value)
}

#[allow(dead_code)]
pub(crate) fn clone_shared<T: ?Sized>(shared: &Shared<T>) -> Shared<T> {
    shared.clone()
}

pub(crate) async fn deadline_after<T, E>(
    duration: Duration,
    future: impl Future<Output = T>,
    on_timeout: impl FnOnce() -> E,
) -> Result<T, E> {
    let work = future.fuse();
    let timer = Delay::new(duration).fuse();
    pin_mut!(work, timer);
    futures::select_biased! {
        result = work => Ok(result),
        () = timer => Err(on_timeout()),
    }
}

pub(crate) fn prefer_primary<T, E>(primary: Result<T, E>, cleanup: Result<(), E>) -> Result<T, E> {
    match primary {
        Err(error) => Err(error),
        Ok(value) => cleanup.map(|()| value),
    }
}
