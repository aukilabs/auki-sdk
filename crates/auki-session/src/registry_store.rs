//! Local registry storage helpers.

use std::collections::HashMap;

#[derive(Debug)]
pub struct RegistryStore<T> {
    entries: HashMap<String, T>,
}

impl<T> Default for RegistryStore<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<T> RegistryStore<T> {
    pub fn insert(&mut self, id: impl Into<String>, entry: T) {
        self.entries.insert(id.into(), entry);
    }

    pub fn get(&self, id: &str) -> Option<&T> {
        self.entries.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &T)> {
        self.entries.iter()
    }
}
