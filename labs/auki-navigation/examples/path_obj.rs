use auki_geometry::parse_obj;
use auki_navigation::{BakeProfile, Extents, NavMesh, Vec3};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: path_obj <file.obj> [radius_m]");
    // metres → voxels. Default 0.05 (1 cell at cs=0.05). Pass 0.0 only to bit-match legacy DSC.
    let radius: f32 = args
        .next()
        .map(|s| s.parse().expect("radius_m"))
        .unwrap_or(0.05);

    let text = std::fs::read_to_string(&path).expect("read obj");
    let mesh = parse_obj(&text).expect("parse");
    eprintln!(
        "mesh: {} verts, {} tris, {} groups; bake radius={radius}",
        mesh.vertices.len(),
        mesh.indices.len(),
        mesh.groups.len()
    );

    let nav = NavMesh::bake(&mesh, &BakeProfile::dsc(radius)).expect("bake");
    let start = Vec3::new(-9.0, 0.0, -0.5);
    let end = Vec3::new(-3.5, 0.0, 1.5);
    let extents = Extents::default();

    let s = nav.find_closest_point(start, extents);
    let e = nav.find_closest_point(end, extents);
    eprintln!(
        "start ({:.3},{:.3},{:.3}) -> ({:.3},{:.3},{:.3}) off={}",
        start.x, start.y, start.z, s.adjusted.x, s.adjusted.y, s.adjusted.z, s.is_off_mesh
    );
    eprintln!(
        "end   ({:.3},{:.3},{:.3}) -> ({:.3},{:.3},{:.3}) off={}",
        end.x, end.y, end.z, e.adjusted.x, e.adjusted.y, e.adjusted.z, e.is_off_mesh
    );

    match nav.find_path(&[start, end]) {
        Ok(p) => {
            eprintln!("path len={:.3} m, {} pts", p.total_distance, p.full.len());
            for (i, pt) in p.full.iter().enumerate() {
                println!("{i:02}: {:.4} {:.4} {:.4}", pt.x, pt.y, pt.z);
            }
        }
        Err(err) => eprintln!("find_path error: {err}"),
    }
}
