//! Explicit application protocol serving configuration.

use auki_protocols::{
    blob::v1 as blobs_v1,
    catalog::{v2 as resources_v2, v3 as resources_v3, v4 as resources_v4},
    info::v1 as info_v1,
    message::v1 as messages_v1,
    registry::{v2 as registries_v2, v3 as registries_v3},
    stream::v2 as streams_v2,
};

/// Exact application protocol versions served by one [`crate::Domain`].
///
/// Client operations are compiled independently of this value. Selecting a
/// protocol here only installs its inbound handler on this Domain instance.
/// The empty/default value serves no application protocols.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ServedProtocols {
    info_v1: bool,
    resources_v2: bool,
    resources_v3: bool,
    resources_v4: bool,
    registries_v2: bool,
    registries_v3: bool,
    blobs_v1: bool,
    messages_v1: bool,
    streams_v2: bool,
}

impl ServedProtocols {
    /// Serve no application protocols.
    pub const fn none() -> Self {
        Self {
            info_v1: false,
            resources_v2: false,
            resources_v3: false,
            resources_v4: false,
            registries_v2: false,
            registries_v3: false,
            blobs_v1: false,
            messages_v1: false,
            streams_v2: false,
        }
    }

    /// Serve participant info v1.0.0.
    pub const fn with_info_v1(mut self) -> Self {
        self.info_v1 = true;
        self
    }

    /// Serve resource catalog v0.2.0.
    pub const fn with_resources_v2(mut self) -> Self {
        self.resources_v2 = true;
        self
    }

    /// Serve resource catalog v0.3.0.
    pub const fn with_resources_v3(mut self) -> Self {
        self.resources_v3 = true;
        self
    }

    /// Serve Map Log resource catalog v0.4.0.
    pub const fn with_resources_v4(mut self) -> Self {
        self.resources_v4 = true;
        self
    }

    /// Serve Registry get v0.2.0.
    pub const fn with_registries_v2(mut self) -> Self {
        self.registries_v2 = true;
        self
    }

    /// Serve Registry list-and-fetch v0.3.0.
    pub const fn with_registries_v3(mut self) -> Self {
        self.registries_v3 = true;
        self
    }

    /// Serve content-addressed blobs v0.1.0.
    pub const fn with_blobs_v1(mut self) -> Self {
        self.blobs_v1 = true;
        self
    }

    /// Serve live messages v0.1.0.
    pub const fn with_messages_v1(mut self) -> Self {
        self.messages_v1 = true;
        self
    }

    /// Serve typed streams v0.2.0.
    pub const fn with_streams_v2(mut self) -> Self {
        self.streams_v2 = true;
        self
    }

    pub(crate) const fn serves_info_v1(self) -> bool {
        self.info_v1
    }

    pub(crate) const fn serves_resources_v2(self) -> bool {
        self.resources_v2
    }

    pub(crate) const fn serves_resources_v3(self) -> bool {
        self.resources_v3
    }

    pub(crate) const fn serves_resources_v4(self) -> bool {
        self.resources_v4
    }

    pub(crate) const fn serves_registries_v2(self) -> bool {
        self.registries_v2
    }

    pub(crate) const fn serves_registries_v3(self) -> bool {
        self.registries_v3
    }

    pub(crate) const fn serves_blobs_v1(self) -> bool {
        self.blobs_v1
    }

    pub(crate) const fn serves_messages_v1(self) -> bool {
        self.messages_v1
    }

    pub(crate) const fn serves_streams_v2(self) -> bool {
        self.streams_v2
    }

    pub(crate) fn protocol_ids(self) -> Vec<&'static str> {
        let mut ids = Vec::with_capacity(9);
        if self.info_v1 {
            ids.push(info_v1::ID);
        }
        if self.resources_v2 {
            ids.push(resources_v2::ID);
        }
        if self.resources_v3 {
            ids.push(resources_v3::ID);
        }
        if self.resources_v4 {
            ids.push(resources_v4::ID);
        }
        if self.registries_v2 {
            ids.push(registries_v2::ID);
        }
        if self.registries_v3 {
            ids.push(registries_v3::ID);
        }
        if self.blobs_v1 {
            ids.push(blobs_v1::ID);
        }
        if self.messages_v1 {
            ids.push(messages_v1::ID);
        }
        if self.streams_v2 {
            ids.push(streams_v2::ID);
        }
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_and_none_serve_nothing() {
        assert!(ServedProtocols::default().protocol_ids().is_empty());
        assert!(ServedProtocols::none().protocol_ids().is_empty());
    }

    #[test]
    fn selection_reports_only_exact_selected_versions() {
        let selected = ServedProtocols::none()
            .with_resources_v3()
            .with_messages_v1()
            .with_streams_v2();
        assert_eq!(
            selected.protocol_ids(),
            [resources_v3::ID, messages_v1::ID, streams_v2::ID]
        );
    }
}
