//! Exact existing registry frame identity plus its resolvable definition.
pub use auki_registry::{FrameRegistryEntry, RegistryRef};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RegisteredFrame {
    pub reference: RegistryRef,
    pub definition: FrameRegistryEntry,
}
impl RegisteredFrame {
    pub fn new(definition: FrameRegistryEntry) -> Self {
        Self {
            reference: RegistryRef {
                peer_id: definition.peer_id.clone(),
                id: definition.frame_id.clone(),
                hash: definition.hash(),
            },
            definition,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let d = &self.definition;
        if d.peer_id.trim().is_empty() {
            return Err("frame owner is required".into());
        }
        FrameRegistryEntry::validate_id(&d.frame_id).map_err(|e| e.to_string())?;
        d.validate().map_err(|e| e.to_string())?;
        if self.reference.peer_id != d.peer_id
            || self.reference.id != d.frame_id
            || self.reference.hash != d.hash()
        {
            return Err("frame registry reference does not match its definition".into());
        }
        Ok(())
    }
}
