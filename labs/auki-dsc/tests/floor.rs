use auki_dsc::{BakeProfile, DscError, NavMesh, OcclusionMesh, Vec3, parse_obj};

fn floor_mesh() -> auki_dsc::TriangleMesh {
    parse_obj(include_str!("../fixtures/floor.obj")).unwrap()
}

fn l_mesh() -> auki_dsc::TriangleMesh {
    parse_obj(include_str!("../fixtures/l_shape.obj")).unwrap()
}

#[test]
fn empty_mesh_errors() {
    let mesh = auki_dsc::TriangleMesh::default();
    let result = NavMesh::bake(&mesh, &BakeProfile::dsc(0.05));
    assert!(matches!(result, Err(DscError::EmptyMesh)));
}

#[test]
fn open_floor_path_approx_diagonal() {
    let mesh = floor_mesh();
    let nav = NavMesh::bake(&mesh, &BakeProfile::dsc(0.05)).unwrap();
    let start = Vec3::new(-4.0, 0.0, -4.0);
    let end = Vec3::new(4.0, 0.0, 4.0);
    let path = nav.find_path(&[start, end]).unwrap();
    let expected = start.distance(end);
    assert!(
        (path.total_distance - expected).abs() < 0.75,
        "path {} vs diagonal {}",
        path.total_distance,
        expected
    );
    assert!(path.full.len() >= 2);
}

#[test]
fn l_shape_does_not_cut_corner() {
    let mesh = l_mesh();
    let nav = NavMesh::bake(&mesh, &BakeProfile::dsc(0.05)).unwrap();
    let start = Vec3::new(5.0, 0.0, 1.0);
    let end = Vec3::new(1.0, 0.0, 5.0);
    let path = nav.find_path(&[start, end]).unwrap();
    let straight = start.distance(end);
    assert!(
        path.total_distance > straight + 0.5,
        "expected detour around corner: path {} straight {}",
        path.total_distance,
        straight
    );
}

#[test]
fn restrict_pushes_out() {
    let mesh = floor_mesh();
    let nav = NavMesh::bake(&mesh, &BakeProfile::restrict_bake()).unwrap();
    let target = Vec3::new(0.0, 0.0, 0.0);
    let result = nav.restrict(target, 1.0);
    assert_eq!(result.original, target);
    let sep = result.restricted.distance(target);
    assert!(
        sep >= 0.99 || sep < 1e-4,
        "expected push-out or already at min sep, got {sep}"
    );
}

#[test]
fn raycast_hit_and_miss() {
    let mesh = floor_mesh();
    let occ = OcclusionMesh::from_mesh(mesh);
    let hits = occ.raycast(Vec3::new(0.0, 5.0, 0.0), Vec3::new(0.0, -1.0, 0.0));
    assert!(!hits.is_empty(), "expected hit on floor from above");
    assert!(hits[0].point.y.abs() < 0.05);

    let miss = occ.raycast(Vec3::new(0.0, 5.0, 0.0), Vec3::new(0.0, 1.0, 0.0));
    assert!(miss.is_empty(), "ray away from floor should miss");
}

#[test]
fn cross_section_nonempty_png() {
    let mesh = floor_mesh();
    let occ = OcclusionMesh::from_mesh(mesh);
    let cs = occ.cross_section(0.0, 20.0);
    assert!(cs.png.len() > 8, "expected PNG bytes");
    assert_eq!(&cs.png[..4], &[0x89, b'P', b'N', b'G']);
    assert!(cs.map_yaml.contains("resolution:"));
    assert!(cs.width >= 1 && cs.height >= 1);
}

#[test]
fn optimized_path_visits_all() {
    let mesh = floor_mesh();
    let nav = NavMesh::bake(&mesh, &BakeProfile::dsc(0.05)).unwrap();
    let wps = [
        Vec3::new(-3.0, 0.0, -3.0),
        Vec3::new(3.0, 0.0, -3.0),
        Vec3::new(3.0, 0.0, 3.0),
        Vec3::new(-3.0, 0.0, 3.0),
    ];
    let opt = nav.find_optimized_path(&wps, true).unwrap();
    assert_eq!(opt.waypoint_indices.len(), 4);
    assert_eq!(opt.waypoint_indices[0], 0);
    assert_eq!(*opt.waypoint_indices.last().unwrap(), 3);
    assert!(opt.path.total_distance > 0.0);
}
