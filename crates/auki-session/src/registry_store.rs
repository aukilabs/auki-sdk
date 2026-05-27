//! Local registry storage helpers.

use std::collections::HashMap;

#[derive(Debug)]
pub struct RegistryStore<T> {
    entries: HashMap<String, T>,
}

impl<T> Default for RegistryStore<T> {
    fn default() -> Self {
        Self { entries: HashMap::new() }
    }
}
