# Auki Portal Mapper

Reference integration between the SDK's detector-neutral Portal Mapper,
`auki-qr-detector`, and Auki's DDS Portal Service.

The component has two intentionally separate boundaries:

1. `adapt_qr_detection_input` converts the reference detector's typed QR
   `DetectionFrame` stream into ordered Portal candidates. Developers using a
   different detector can produce `auki_mappers::PortalDetectionBatch`
   directly and do not depend on this crate.
2. `DdsPortalResolver` recognizes production `https://r8.hr/{short_id}` QR
   payloads, retrieves their canonical lighthouse record, converts DDS's
   centimetre size to metres, and caches the result for a bounded interval.

For Bracketbot, construct `PortalMapperRunner` with
`from_sdk_pose_chain_contract`: the robot publishes the rigid
`head_cam_optical → base_link` edge and the live `base_link → map` SLAM Pose
Log. The runner composes those edges before interpolating camera pose at each
detection timestamp.

The host application supplies the DDS base URL, current Authorization header,
and `posemesh-client-id`; credentials are not stored in SDK registries or map
updates.
