use auki_geometry::{parse_obj, TriangleMesh};
use auki_navigation::{BakeProfile, NavError, NavMesh, Vec3};

fn floor_mesh() -> TriangleMesh {
    parse_obj(include_str!("../fixtures/floor.obj")).unwrap()
}

fn l_mesh() -> TriangleMesh {
    parse_obj(include_str!("../fixtures/l_shape.obj")).unwrap()
}

#[test]
fn empty_mesh_errors() {
    let mesh = TriangleMesh::default();
    let result = NavMesh::bake(&mesh, &BakeProfile::dsc(0.05));
    assert!(matches!(result, Err(NavError::EmptyMesh)));
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
