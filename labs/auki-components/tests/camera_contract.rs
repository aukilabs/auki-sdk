use auki_components::{CameraPayloadContract, PayloadContract};

#[test]
fn camera_payload_contract_serializes_nominal_frame_rate() {
    let contract = PayloadContract::Camera(CameraPayloadContract {
        datatype: "bytes".into(),
        schema: "image/jpeg".into(),
        encoding: "jpeg".into(),
        width: 640,
        height: 360,
        nominal_frame_rate_hz: Some(10),
        observes: "browser webcam".into(),
    });

    let json = serde_json::to_value(contract).expect("camera contract serializes");
    assert_eq!(json["nominal_frame_rate_hz"], 10);
}

#[test]
fn camera_payload_contract_can_declare_unknown_nominal_rate() {
    let json = serde_json::json!({
        "kind": "camera",
        "datatype": "video_frame",
        "schema": "auki.video-frame/v1",
        "encoding": "rgb8",
        "width": 640,
        "height": 480,
        "nominal_frame_rate_hz": null,
        "observes": "visible_light"
    });

    let contract: PayloadContract = serde_json::from_value(json).expect("camera contract parses");
    let PayloadContract::Camera(camera) = contract else {
        panic!("expected camera contract");
    };
    assert_eq!(camera.nominal_frame_rate_hz, None);
}
