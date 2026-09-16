use crate::{DataError, DomainDataClient, PortalId, PortalPose};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

impl DomainDataClient {
    pub async fn poses(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<PortalPose>, DataError> {
        #[derive(Deserialize)]
        struct Response {
            poses: Vec<PortalPose>,
        }
        let response: Response = self
            .json_request(
                |http, access| {
                    http.get(
                        access
                            .server_url()
                            .join(&format!("api/v1/domains/{}/lighthouses", self.domain_id()))
                            .expect("UUID path"),
                    )
                },
                cancellation,
            )
            .await?;
        if response
            .poses
            .iter()
            .any(|pose| pose.domain_id != self.domain_id())
        {
            return Err(DataError::InvalidResponse("pose belongs to another Domain"));
        }
        Ok(response.poses)
    }

    pub async fn pose(
        &self,
        portal: &PortalId,
        cancellation: &CancellationToken,
    ) -> Result<PortalPose, DataError> {
        let pose: PortalPose = self
            .json_request(
                |http, access| {
                    http.get(
                        access
                            .server_url()
                            .join(&format!(
                                "api/v1/domains/{}/lighthouses/{}",
                                self.domain_id(),
                                portal.as_str()
                            ))
                            .expect("validated portal path"),
                    )
                },
                cancellation,
            )
            .await?;
        if pose.domain_id != self.domain_id() || !portal.matches(pose.id, &pose.short_id) {
            return Err(DataError::InvalidResponse("wrong Domain or portal pose"));
        }
        Ok(pose)
    }
}
