use crate::core;
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
    core::add(left, right)
}

#[uniffi::export]
pub fn hello(name: String) -> String {
    core::hello(&name)
}

#[uniffi::export]
pub fn make_greeting(name: String, style: GreetingStyle) -> Result<Greeting, TestError> {
    core::make_greeting(&name, style.into())
        .map(Into::into)
        .map_err(Into::into)
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn delayed_greeting(name: String, delay_ms: u32) -> Result<Greeting, TestError> {
    core::validate_delay(delay_ms).map_err(TestError::from)?;
    if delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(u64::from(delay_ms))).await;
    }
    core::make_greeting(&name, core::GreetingStyle::Casual)
        .map(Into::into)
        .map_err(Into::into)
}

#[derive(uniffi::Object, Debug)]
pub struct Counter {
    value: Mutex<core::CounterState>,
}

#[uniffi::export]
impl Counter {
    #[uniffi::constructor]
    pub fn new(initial: i32) -> Arc<Self> {
        Arc::new(Self {
            value: Mutex::new(core::CounterState::new(initial)),
        })
    }

    pub fn value(&self) -> i32 {
        self.value.lock().expect("counter mutex poisoned").value()
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Counter {
    pub async fn add_after(&self, delta: i32, delay_ms: u32) -> Result<i32, TestError> {
        core::validate_delay(delay_ms).map_err(TestError::from)?;
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(u64::from(delay_ms))).await;
        }

        let mut value = self.value.lock().expect("counter mutex poisoned");
        Ok(value.add(delta))
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
        Self {
            message: greeting.message,
            name_length: greeting.name_length,
            style: greeting.style.into(),
        }
    }
}

impl From<core::TestError> for TestError {
    fn from(err: core::TestError) -> Self {
        match err {
            core::TestError::EmptyName => Self::EmptyName,
            core::TestError::DelayTooLarge { max_ms } => Self::DelayTooLarge { max_ms },
        }
    }
}
