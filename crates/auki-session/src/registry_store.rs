//! Local registry storage helpers.

use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct RegistryStore<T> {
    entries: HashMap<String, T>,
}
