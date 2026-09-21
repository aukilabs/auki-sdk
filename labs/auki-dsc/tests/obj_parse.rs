use auki_dsc::{DscError, parse_obj};

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
    assert!(matches!(err, DscError::EmptyMesh));
}
