//! Small UniFFI test crate.
//!
//! This crate is intentionally not an SDK component. It is a compact
//! proving ground for UniFFI proc-macro exports before the real Auki
//! crates get Swift bindings.

use std::sync::{Arc, Mutex};
use std::time::Duration;

uniffi::setup_scaffolding!();

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum GreetingStyle {
    Casual,
    Formal,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct Greeting {
    pub message: String,
    pub name_length: u32,
    pub style: GreetingStyle,
}

#[derive(uniffi::Error, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TestError {
    #[error("name must not be empty")]
    EmptyName,
    #[error("delay is too large; max {max_ms} ms")]
    DelayTooLarge { max_ms: u32 },
}

#[uniffi::export]
pub fn add(left: i32, right: i32) -> i32 {
    left + right
}

#[uniffi::export]
pub fn hello(name: String) -> String {
    format!("Hello, {name}.")
}

#[uniffi::export]
pub fn make_greeting(name: String, style: GreetingStyle) -> Result<Greeting, TestError> {
    greeting(name, style)
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn delayed_greeting(name: String, delay_ms: u32) -> Result<Greeting, TestError> {
    validate_delay(delay_ms)?;
    if delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(u64::from(delay_ms))).await;
    }
    greeting(name, GreetingStyle::Casual)
}

#[derive(uniffi::Object, Debug)]
pub struct Counter {
    value: Mutex<i32>,
}

#[uniffi::export]
impl Counter {
    #[uniffi::constructor]
    pub fn new(initial: i32) -> Arc<Self> {
        Arc::new(Self {
            value: Mutex::new(initial),
        })
    }

    pub fn value(&self) -> i32 {
        *self.value.lock().expect("counter mutex poisoned")
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Counter {
    pub async fn add_after(&self, delta: i32, delay_ms: u32) -> Result<i32, TestError> {
        validate_delay(delay_ms)?;
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(u64::from(delay_ms))).await;
        }

        let mut value = self.value.lock().expect("counter mutex poisoned");
        *value += delta;
        Ok(*value)
    }
}

fn greeting(name: String, style: GreetingStyle) -> Result<Greeting, TestError> {
    if name.is_empty() {
        return Err(TestError::EmptyName);
    }

    let message = match style {
        GreetingStyle::Casual => format!("Hello, {name}."),
        GreetingStyle::Formal => format!("Good day, {name}."),
    };
    let name_length = name.chars().count() as u32;

    Ok(Greeting {
        message,
        name_length,
        style,
    })
}

fn validate_delay(delay_ms: u32) -> Result<(), TestError> {
    const MAX_DELAY_MS: u32 = 1_000;
    if delay_ms > MAX_DELAY_MS {
        return Err(TestError::DelayTooLarge {
            max_ms: MAX_DELAY_MS,
        });
    }
    Ok(())
}
