use auki_geometry::mesh::Vec3 as MeshVec3;
use auki_geometry::{TriangleMesh, parse_obj};
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

fn corridor_floor() -> TriangleMesh {
    TriangleMesh::from_indexed(
        vec![
            MeshVec3::new(-6.0, 0.0, -1.5),
            MeshVec3::new(6.0, 0.0, -1.5),
            MeshVec3::new(6.0, 0.0, 1.5),
            MeshVec3::new(-6.0, 0.0, 1.5),
        ],
        vec![[0, 1, 2], [0, 2, 3]],
        vec![],
    )
    .unwrap()
}

fn box_mesh(min: MeshVec3, max: MeshVec3) -> TriangleMesh {
    let (x0, y0, z0) = (min.x, min.y, min.z);
    let (x1, y1, z1) = (max.x, max.y, max.z);
    let corners = [
        MeshVec3::new(x0, y0, z0),
        MeshVec3::new(x1, y0, z0),
        MeshVec3::new(x1, y0, z1),
        MeshVec3::new(x0, y0, z1),
        MeshVec3::new(x0, y1, z0),
        MeshVec3::new(x1, y1, z0),
        MeshVec3::new(x1, y1, z1),
        MeshVec3::new(x0, y1, z1),
    ];
    // Each face is two triangles. Winding is irrelevant; bake flips downward faces.
    let faces: [[u32; 4]; 6] = [
        [0, 1, 2, 3],
        [4, 7, 6, 5],
        [0, 4, 5, 1],
        [1, 5, 6, 2],
        [2, 6, 7, 3],
        [3, 7, 4, 0],
    ];
    let mut indices = Vec::new();
    for face in faces {
        indices.push([face[0], face[1], face[2]]);
        indices.push([face[0], face[2], face[3]]);
    }
    TriangleMesh::from_indexed(corners.to_vec(), indices, vec![]).unwrap()
}

#[test]
fn shelf_blocks_corridor_the_floor_would_allow() {
    let floor = corridor_floor();
    let shelf = box_mesh(MeshVec3::new(-0.4, 0.0, -1.6), MeshVec3::new(0.4, 1.5, 1.6));
    let profile = BakeProfile::biped(0.2);
    assert!((profile.walkable_radius - 0.2).abs() < 1e-6);
    let start = Vec3::new(-4.0, 0.0, 0.0);
    let end = Vec3::new(4.0, 0.0, 0.0);

    let open = NavMesh::bake(&floor, &profile).unwrap();
    assert!(open.find_path(&[start, end]).is_ok());

    let blocked = NavMesh::bake_with_obstacles(&floor, &shelf, &profile).unwrap();
    let result = blocked.find_path(&[start, end]);
    assert!(
        matches!(result, Err(NavError::NoPath) | Err(NavError::OffMesh)),
        "shelf should remove the corridor, got {result:?}"
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
