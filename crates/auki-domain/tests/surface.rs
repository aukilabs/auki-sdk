use auki_domain::{
    ClusterMember, ClusterMembership, cluster_membership_admit_member_json,
    cluster_membership_filename_json, cluster_membership_new_json,
    cluster_membership_peer_count_json, elect_successor_json,
};
use libp2p_identity::PeerId;

#[test]
fn rust_root_api_remains_source_compatible() {
    let peer_a: PeerId = "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"
        .parse()
        .unwrap();
    let peer_b: PeerId = "12D3KooWEkBHD6M9cqgwhwMnAb6eznr1T3c98KHGQGYVzu9c7cgw"
        .parse()
        .unwrap();

    let mut membership = ClusterMembership::new("demo");
    membership.admit(ClusterMember {
        peer_id: peer_a,
        multiaddrs: vec!["/ip4/127.0.0.1/tcp/4001".parse().unwrap()],
        join_ts_ns: 10,
        successor_token: None,
    });
    membership.admit(ClusterMember {
        peer_id: peer_b,
        multiaddrs: vec!["/ip4/127.0.0.1/tcp/4002".parse().unwrap()],
        join_ts_ns: 20,
        successor_token: None,
    });
    assert_eq!(membership.filename(), "demo.json");

    let json = cluster_membership_new_json("demo");
    let json = cluster_membership_admit_member_json(
        &json,
        r#"{"peer_id":"12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar","multiaddrs":["/ip4/127.0.0.1/tcp/4001"],"join_ts_ns":10}"#,
    )
    .unwrap();
    assert_eq!(
        cluster_membership_filename_json(&json).unwrap(),
        "demo.json"
    );
    assert_eq!(cluster_membership_peer_count_json(&json).unwrap(), 1);
    assert_eq!(
        elect_successor_json(
            &json,
            "12D3KooWEkBHD6M9cqgwhwMnAb6eznr1T3c98KHGQGYVzu9c7cgw",
            vec!["12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar".into()],
        )
        .unwrap()
        .as_deref(),
        Some("12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar")
    );
}
