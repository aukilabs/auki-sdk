//! Convenience duplication into a named convention. Selection retains the map origin.
use crate::{
    CoordinateConvention, MapConventionConversion, MapFrame, MapSnapshot, SceneError, Scenegraph,
};
use std::collections::BTreeSet;

/// Destination identity and coordinate convention for a new, independent artifact.
/// Named destinations default to meters; `meters_per_unit` can be set explicitly.
#[derive(Clone, Debug)]
pub struct MapDuplicateTarget {
    pub map_id: String,
    pub frame_id: String,
    pub convention: CoordinateConvention,
    pub meters_per_unit: f64,
}
impl MapDuplicateTarget {
    pub fn new(
        map_id: impl Into<String>,
        frame_id: impl Into<String>,
        convention: CoordinateConvention,
    ) -> Self {
        Self {
            map_id: map_id.into(),
            frame_id: frame_id.into(),
            convention,
            meters_per_unit: 1.,
        }
    }
}

impl Scenegraph {
    /// Duplicate the whole scenegraph. Requires an explicitly named source convention.
    /// The source, physical Portal identities and physical origin remain unchanged.
    pub fn duplicate_in_convention(&self, target: &MapDuplicateTarget) -> Result<Self, SceneError> {
        self.duplicate_selection(None, target)
    }

    /// Duplicate selected direct Portal children under a copy of the map root.
    /// This is an explicit selection, not a rebase onto the first selected Portal.
    /// Unknown or repeated IDs fail atomically. An empty selection copies only the root.
    pub fn duplicate_part_in_convention(
        &self,
        anchor_ids: &[&str],
        target: &MapDuplicateTarget,
    ) -> Result<Self, SceneError> {
        self.duplicate_selection(Some(anchor_ids), target)
    }

    fn duplicate_selection(
        &self,
        anchor_ids: Option<&[&str]>,
        target: &MapDuplicateTarget,
    ) -> Result<Self, SceneError> {
        self.validate()?;
        let target_frame = MapFrame::in_convention(
            &target.frame_id,
            format!(
                "Same physical origin as {}; coordinates expressed in {}",
                self.map.frame.id,
                target.convention.as_str()
            ),
            target.convention,
            target.meters_per_unit,
        )?;
        let conversion =
            MapConventionConversion::between_named_frames(self.map.frame.clone(), target_frame)?;
        let mut selected = Scenegraph::new(self.map.clone())?;
        match anchor_ids {
            None => selected.anchors = self.anchors.clone(),
            Some(ids) => {
                let mut unique = BTreeSet::new();
                for id in ids {
                    if !unique.insert(*id) {
                        return Err(SceneError::Invalid(
                            "duplicate anchor ID in scenegraph selection",
                        ));
                    }
                    let anchor = self.anchors.get(*id).ok_or(SceneError::Invalid(
                        "unknown anchor ID in scenegraph selection",
                    ))?;
                    selected.anchors.insert((*id).into(), anchor.clone());
                }
            }
        }
        if selected
            .anchors
            .values()
            .any(|a| a.pose_in_map.from_frame_id == target.frame_id)
        {
            return Err(SceneError::Invalid(
                "target map frame collides with a Portal-local frame",
            ));
        }
        conversion.convert_scenegraph(&selected, &target.map_id)
    }
}

impl MapSnapshot {
    /// Duplicate a validated snapshot and regenerate canonical USDA.
    pub fn duplicate_in_convention(&self, target: &MapDuplicateTarget) -> Result<Self, SceneError> {
        self.validate()?;
        Self::new(self.scenegraph.duplicate_in_convention(target)?)
    }

    /// Duplicate a selection and regenerate USDA containing only those Portals.
    pub fn duplicate_part_in_convention(
        &self,
        anchor_ids: &[&str],
        target: &MapDuplicateTarget,
    ) -> Result<Self, SceneError> {
        self.validate()?;
        Self::new(
            self.scenegraph
                .duplicate_part_in_convention(anchor_ids, target)?,
        )
    }
}
