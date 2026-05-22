#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GreetingStyle {
    Casual,
    Formal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Greeting {
    pub message: String,
    pub name_length: u32,
    pub style: GreetingStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestError {
    EmptyName,
    DelayTooLarge { max_ms: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CounterState {
    value: i32,
}

impl CounterState {
    pub fn new(initial: i32) -> Self {
        Self { value: initial }
    }

    pub fn value(&self) -> i32 {
        self.value
    }

    pub fn add(&mut self, delta: i32) -> i32 {
        self.value += delta;
        self.value
    }
}

pub fn add(left: i32, right: i32) -> i32 {
    left + right
}

pub fn hello(name: &str) -> String {
    format!("Hello, {name}.")
}

pub fn make_greeting(name: &str, style: GreetingStyle) -> Result<Greeting, TestError> {
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

pub fn validate_delay(delay_ms: u32) -> Result<(), TestError> {
    const MAX_DELAY_MS: u32 = 1_000;
    if delay_ms > MAX_DELAY_MS {
        return Err(TestError::DelayTooLarge {
            max_ms: MAX_DELAY_MS,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_plain_values_match_current_surface() {
        assert_eq!(add(2, 3), 5);
        assert_eq!(hello("Auki"), "Hello, Auki.");
    }

    #[test]
    fn core_greeting_validates_empty_name() {
        let err = make_greeting("", GreetingStyle::Casual).expect_err("empty name rejected");
        assert_eq!(err, TestError::EmptyName);
    }

    #[test]
    fn core_greeting_tracks_style_and_length() {
        let greeting = make_greeting("Auki", GreetingStyle::Formal).expect("valid greeting");
        assert_eq!(greeting.message, "Good day, Auki.");
        assert_eq!(greeting.name_length, 4);
        assert_eq!(greeting.style, GreetingStyle::Formal);
    }

    #[test]
    fn core_counter_holds_state() {
        let mut counter = CounterState::new(10);
        assert_eq!(counter.value(), 10);
        assert_eq!(counter.add(7), 17);
        assert_eq!(counter.value(), 17);
    }

    #[test]
    fn core_delay_validation_rejects_large_delay() {
        assert_eq!(
            validate_delay(1_001),
            Err(TestError::DelayTooLarge { max_ms: 1_000 })
        );
    }
}
