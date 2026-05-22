use crate::core;
use gloo_timers::future::TimeoutFuture;
use js_sys::{Error, Promise};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

#[wasm_bindgen]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GreetingStyle {
    Casual,
    Formal,
}

#[wasm_bindgen]
pub struct Greeting {
    inner: core::Greeting,
}

#[wasm_bindgen]
impl Greeting {
    #[wasm_bindgen(getter)]
    pub fn message(&self) -> String {
        self.inner.message.clone()
    }

    #[wasm_bindgen(getter, js_name = nameLength)]
    pub fn name_length(&self) -> u32 {
        self.inner.name_length
    }

    #[wasm_bindgen(getter)]
    pub fn style(&self) -> GreetingStyle {
        self.inner.style.clone().into()
    }
}

#[wasm_bindgen]
pub struct Counter {
    value: Rc<RefCell<core::CounterState>>,
}

#[wasm_bindgen]
impl Counter {
    #[wasm_bindgen(constructor)]
    pub fn new(initial: i32) -> Self {
        Self {
            value: Rc::new(RefCell::new(core::CounterState::new(initial))),
        }
    }

    pub fn value(&self) -> i32 {
        self.value.borrow().value()
    }

    #[wasm_bindgen(js_name = addAfter)]
    pub fn add_after(&self, delta: i32, delay_ms: u32) -> Promise {
        let value = Rc::clone(&self.value);
        future_to_promise(async move {
            add_after_task(value, delta, delay_ms)
                .await
                .map(JsValue::from)
        })
    }
}

#[wasm_bindgen]
pub fn add(left: i32, right: i32) -> i32 {
    core::add(left, right)
}

#[wasm_bindgen]
pub fn hello(name: String) -> String {
    core::hello(&name)
}

#[wasm_bindgen(js_name = makeGreeting)]
pub fn make_greeting(name: String, style: GreetingStyle) -> Result<Greeting, JsValue> {
    core::make_greeting(&name, style.into())
        .map(Into::into)
        .map_err(test_error_to_js)
}

#[wasm_bindgen(js_name = delayedGreeting)]
pub async fn delayed_greeting(name: String, delay_ms: u32) -> Result<Greeting, JsValue> {
    delay(delay_ms).await?;
    core::make_greeting(&name, core::GreetingStyle::Casual)
        .map(Into::into)
        .map_err(test_error_to_js)
}

async fn delay(delay_ms: u32) -> Result<(), JsValue> {
    core::validate_delay(delay_ms).map_err(test_error_to_js)?;
    if delay_ms > 0 {
        TimeoutFuture::new(delay_ms).await;
    }
    Ok(())
}

async fn add_after_task(
    value: Rc<RefCell<core::CounterState>>,
    delta: i32,
    delay_ms: u32,
) -> Result<i32, JsValue> {
    delay(delay_ms).await?;
    Ok(value.borrow_mut().add(delta))
}

fn test_error_to_js(err: core::TestError) -> JsValue {
    Error::new(&test_error_message(err)).into()
}

fn test_error_message(err: core::TestError) -> String {
    match err {
        core::TestError::EmptyName => "name must not be empty".to_string(),
        core::TestError::DelayTooLarge { max_ms } => {
            format!("delay is too large; max {max_ms} ms")
        }
    }
}

impl From<GreetingStyle> for core::GreetingStyle {
    fn from(style: GreetingStyle) -> Self {
        match style {
            GreetingStyle::Casual => Self::Casual,
            GreetingStyle::Formal => Self::Formal,
        }
    }
}

impl From<core::GreetingStyle> for GreetingStyle {
    fn from(style: core::GreetingStyle) -> Self {
        match style {
            core::GreetingStyle::Casual => Self::Casual,
            core::GreetingStyle::Formal => Self::Formal,
        }
    }
}

impl From<core::Greeting> for Greeting {
    fn from(greeting: core::Greeting) -> Self {
        Self { inner: greeting }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasm_sync_surface_matches_core_behavior() {
        assert_eq!(add(2, 40), 42);
        assert_eq!(hello("Auki".to_string()), "Hello, Auki.");
    }

    #[test]
    fn counter_add_after_future_outlives_js_wrapper() {
        let counter = Counter::new(10);
        let future = add_after_task(Rc::clone(&counter.value), 5, 0);

        drop(counter);
        drop(future);
    }

    #[test]
    fn wasm_error_messages_match_uniffi_display() {
        assert_eq!(
            test_error_message(core::TestError::EmptyName),
            "name must not be empty"
        );
        assert_eq!(
            test_error_message(core::TestError::DelayTooLarge { max_ms: 1_000 }),
            "delay is too large; max 1000 ms"
        );
    }
}
