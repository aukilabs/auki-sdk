use auki_geometry::TriangleMesh;
use auki_geometry::mesh::Vec3;
use auki_raster::cross_section;

fn floor() -> TriangleMesh {
    TriangleMesh::from_indexed(
        vec![
            Vec3::new(-1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(-1.0, 0.0, 1.0),
        ],
        vec![[0, 1, 2], [0, 2, 3]],
        vec![],
    )
    .unwrap()
}

#[test]
fn cross_section_nonempty_png() {
    let cs = cross_section(&floor(), 0.0, 20.0);
    assert!(cs.png.len() > 8, "expected PNG bytes");
    assert_eq!(&cs.png[..4], &[0x89, b'P', b'N', b'G']);
    assert!(cs.map_yaml.contains("resolution:"));
    assert!(cs.width >= 1 && cs.height >= 1);
}
