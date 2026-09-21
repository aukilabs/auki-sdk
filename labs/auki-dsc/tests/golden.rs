//! Golden measurement vs DSC Deno JSON fixtures.
//!
//! Drop a frozen OBJ + DSC JSON under `fixtures/golden/` and remove `#[ignore]`
//! once captured. Soft assert: Hausdorff / max waypoint delta < 0.5 m.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct GoldenPoint {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Debug, Deserialize)]
struct GoldenPath {
    #[serde(default)]
    full: Vec<GoldenPoint>,
}

fn hausdorff(a: &[(f32, f32, f32)], b: &[(f32, f32, f32)]) -> f32 {
    fn directed(from: &[(f32, f32, f32)], to: &[(f32, f32, f32)]) -> f32 {
        from.iter()
            .map(|p| {
                to.iter()
                    .map(|q| {
                        let dx = p.0 - q.0;
                        let dy = p.1 - q.1;
                        let dz = p.2 - q.2;
                        (dx * dx + dy * dy + dz * dz).sqrt()
                    })
                    .fold(f32::INFINITY, f32::min)
            })
            .fold(0.0_f32, f32::max)
    }
    directed(a, b).max(directed(b, a))
}

#[test]
#[ignore = "commit fixtures/golden/{mesh.obj,findpath.json} from DSC Deno first"]
fn golden_findpath_measurement() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/golden");
    let obj = std::fs::read_to_string(root.join("mesh.obj")).expect("mesh.obj");
    let json = std::fs::read_to_string(root.join("findpath.json")).expect("findpath.json");
    let golden: GoldenPath = serde_json::from_str(&json).unwrap();

    let mesh = auki_dsc::parse_obj(&obj).unwrap();
    let nav = auki_dsc::NavMesh::bake(&mesh, &auki_dsc::BakeProfile::dsc(0.05)).unwrap();

    let wps: Vec<_> = golden
        .full
        .first()
        .zip(golden.full.last())
        .map(|(a, b)| {
            [
                auki_dsc::Vec3::new(a.x, a.y, a.z),
                auki_dsc::Vec3::new(b.x, b.y, b.z),
            ]
        })
        .expect("golden needs ≥2 points")
        .to_vec();

    let path = nav.find_path(&wps).unwrap();
    let ours: Vec<_> = path.full.iter().map(|p| (p.x, p.y, p.z)).collect();
    let theirs: Vec<_> = golden.full.iter().map(|p| (p.x, p.y, p.z)).collect();
    let h = hausdorff(&ours, &theirs);
    eprintln!("golden findpath Hausdorff = {h:.3} m");
    assert!(h < 0.5, "Hausdorff {h} exceeds soft gate 0.5 m");
}
