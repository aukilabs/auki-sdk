use auki_geometry::{GeometryError, TriangleMesh, parse_obj};

#[test]
fn triangle_fan_quad() {
    let obj = r#"
v 0 0 0
v 1 0 0
v 1 0 1
v 0 0 1
f 1 2 3 4
"#;
    let mesh = parse_obj(obj).unwrap();
    assert_eq!(mesh.vertices.len(), 4);
    assert_eq!(mesh.indices.len(), 2);
    assert_eq!(mesh.indices[0], [0, 1, 2]);
    assert_eq!(mesh.indices[1], [0, 2, 3]);
}

#[test]
fn named_groups() {
    let obj = r#"
o corridor_a
v 0 0 0
v 1 0 0
v 1 0 1
f 1 2 3
o corridor_b
v 2 0 0
v 3 0 0
v 3 0 1
f 4 5 6
"#;
    let mesh = parse_obj(obj).unwrap();
    assert_eq!(mesh.groups.len(), 2);
    assert_eq!(mesh.groups[0].name, "corridor_a");
    assert_eq!(mesh.groups[1].name, "corridor_b");
    assert_eq!(mesh.groups[0].triangle_range, 0..1);
    assert_eq!(mesh.groups[1].triangle_range, 1..2);
}

#[test]
fn empty_obj_errors() {
    let err = parse_obj("# nothing\n").unwrap_err();
    assert!(matches!(err, GeometryError::EmptyMesh));
}

#[test]
fn from_indexed_skips_obj() {
    use auki_geometry::mesh::Vec3;
    let mesh = TriangleMesh::from_indexed(
        vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ],
        vec![[0, 1, 2]],
        vec![],
    )
    .unwrap();
    assert_eq!(mesh.vertices.len(), 3);
    assert_eq!(mesh.indices.len(), 1);
    assert!(TriangleMesh::from_indexed(vec![], vec![], vec![]).is_err());
}

#[test]
fn raycast_hit_and_miss() {
    use auki_geometry::mesh::Vec3;
    let mesh = TriangleMesh::from_indexed(
        vec![
            Vec3::new(-1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(-1.0, 0.0, 1.0),
        ],
        vec![[0, 1, 2], [0, 2, 3]],
        vec![],
    )
    .unwrap();
    let hits = mesh.raycast(Vec3::new(0.0, 5.0, 0.0), Vec3::new(0.0, -1.0, 0.0));
    assert!(!hits.is_empty(), "expected hit on floor from above");
    assert!(hits[0].point.y.abs() < 0.05);

    let miss = mesh.raycast(Vec3::new(0.0, 5.0, 0.0), Vec3::new(0.0, 1.0, 0.0));
    assert!(miss.is_empty(), "ray away from floor should miss");
}
