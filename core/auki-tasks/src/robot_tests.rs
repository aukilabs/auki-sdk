use super::*;
use serde_json::json;

fn config(dds: &str, audience: Option<&str>) -> Result<RobotConfig> {
    RobotConfig::new(
        dds,
        "http://127.0.0.1:9",
        SecretString::new("fixture-registration"),
        "1.0.0",
        "robot-fixture",
        audience,
        vec!["/example/v1".into()],
    )
}

#[test]
fn robot_audience_defaults_match_official_dds_environments() {
    for (dds, expected) in [
        (
            "https://dds.dev.aukiverse.com",
            "https://dds.dev.aukiverse.com/robots",
        ),
        (
            "https://dds.staging.aukiverse.com/",
            "https://dds.staging.aukiverse.com/robots",
        ),
        (
            "https://dds.auki.network",
            "https://dds.auki.network/robots",
        ),
        (
            "https://DDS.DEV.AUKIVERSE.COM:443/",
            "https://dds.dev.aukiverse.com/robots",
        ),
    ] {
        assert_eq!(config(dds, None).unwrap().audience, expected);
    }
}

#[test]
fn custom_or_modified_dds_endpoints_require_an_explicit_robot_audience() {
    for dds in [
        "https://dds.example.com",
        "http://127.0.0.1:9",
        "https://dds.dev.aukiverse.com.example.com",
        "https://dds.dev.aukiverse.com:8443",
        "https://dds.dev.aukiverse.com/custom",
        "https://dds.dev.aukiverse.com/?environment=dev",
        "https://dds.dev.aukiverse.com/#dev",
        "https://user@dds.dev.aukiverse.com/",
        "http://dds.dev.aukiverse.com",
    ] {
        assert!(matches!(
            config(dds, None),
            Err(TaskError::Configuration(_))
        ));
    }
}

#[test]
fn explicit_robot_audiences_override_presets_and_allow_custom_endpoints() {
    for dds in ["https://dds.dev.aukiverse.com", "http://127.0.0.1:9"] {
        assert_eq!(
            config(dds, Some("custom-robot-audience")).unwrap().audience,
            "custom-robot-audience"
        );
    }
}

#[test]
fn invalid_explicit_audiences_do_not_fall_back_to_a_preset() {
    for audience in [
        "",
        " ",
        " robot-audience",
        "robot audience",
        &"x".repeat(513),
    ] {
        assert!(matches!(
            config("https://dds.dev.aukiverse.com", Some(audience)),
            Err(TaskError::Configuration(_))
        ));
    }
}

#[tokio::test]
async fn default_robot_audience_still_rejects_wrong_or_mixed_token_profiles() {
    let robot =
        AukiRobotCredential::new(config("https://dds.dev.aukiverse.com", None).unwrap()).unwrap();
    let node = Uuid::new_v4();
    let now = Utc::now();
    let expires = now + chrono::Duration::seconds(120);
    let claims = json!({
        "iss": "dds", "aud": ["https://dds.dev.aukiverse.com/robots"],
        "node_type": "robot", "node_mode": "dedicated", "sub": node, "node_id": node,
        "organization_id": Uuid::new_v4(), "assigned_domain_id": Uuid::new_v4(),
        "iat": now.timestamp(), "exp": expires.timestamp()
    });
    // Only exercise the SDK's profile check on an authenticated response here.
    // Signature verification is covered by the existing DDS/P2P contract tests.
    let bundle = |claims: &serde_json::Value| {
        AccessBundle::new(
            format!(
                "e30.{}.fixture",
                URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap())
            ),
            expires,
        )
    };
    assert!(robot.0.auth.validate(&bundle(&claims), false).is_ok());
    for change in [
        json!({"aud": ["https://dds.dev.aukiverse.com"]}),
        json!({"aud": ["https://dds.staging.aukiverse.com/robots"]}),
        json!({"aud": ["https://dds.dev.aukiverse.com/robots", "https://dds.dev.aukiverse.com"]}),
        json!({"iss": "other"}),
        json!({"node_type": "compute"}),
        json!({"exp": 1}),
        json!({"sub": Uuid::new_v4()}),
        json!({"assigned_domain_id": Uuid::new_v4()}),
    ] {
        let mut invalid = claims.clone();
        invalid
            .as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        assert!(matches!(
            robot.0.auth.validate(&bundle(&invalid), false),
            Err(TaskError::Authority(_))
        ));
    }
    robot.close().await;
}
