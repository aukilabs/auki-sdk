"""Optional OpenUSD validation of the mapping experiment artifacts.

python tests/validate_experiment_usd.py ../../target/portal-experiments
Requires usd-core; does not contact services or modify the USDA.
"""
import json
import math
from pathlib import Path
import sys
from pxr import Gf, Usd, UsdGeom

root = Path(sys.argv[1])
files = sorted(p for p in root.glob("0*.usda") if not p.name.startswith("06-"))
assert len(files) == 5
final_polygons = []
for path, expected_count in zip(files, [1, 2, 2, 2, 3]):
    stage = Usd.Stage.Open(str(path))
    assert stage and stage.GetDefaultPrim().GetName() == "Map"
    assert UsdGeom.GetStageUpAxis(stage) == UsdGeom.Tokens.z
    assert UsdGeom.GetStageMetersPerUnit(stage) == 1.0
    scene = json.loads(path.with_suffix(".json").read_text())
    meshes = [p for p in stage.Traverse() if p.IsA(UsdGeom.Mesh)]
    assert len(meshes) == expected_count
    for prim in meshes:
        mesh = UsdGeom.Mesh(prim)
        anchor = prim.GetParent()
        portal_id = anchor.GetAttribute("auki:anchorId").Get()
        expected = scene["anchors"][portal_id]
        assert anchor.GetAttribute("auki:toFrameId").Get() == scene["map"]["frame"]["id"]
        assert list(mesh.GetFaceVertexCountsAttr().Get()) == [4]
        assert list(mesh.GetFaceVertexIndicesAttr().Get()) == [0, 1, 2, 3]
        assert mesh.GetDoubleSidedAttr().Get()
        assert list(mesh.GetDisplayColorAttr().Get()) == [Gf.Vec3f(1, 1, 1)]
        points = mesh.GetPointsAttr().Get()
        size = (points[1] - points[0]).GetLength()
        assert math.isclose(size, expected["side_length_m"], abs_tol=1e-6)
        matrix = UsdGeom.Xformable(prim).ComputeLocalToWorldTransform(Usd.TimeCode.Default())
        center = matrix.Transform(Gf.Vec3d(0, 0, 0))
        assert Gf.IsClose(center, Gf.Vec3d(*expected["pose_in_map"]["translation"]), 1e-9)
        if expected_count == 3:
            n = int(portal_id[-1])
            assert Gf.IsClose(center, Gf.Vec3d((n-1)*2, 0, 1.5), 0.025)
            # Outward face normal points toward the cameras on the -Y side.
            assert Gf.IsClose(matrix.TransformDir(Gf.Vec3d(0, 0, 1)), Gf.Vec3d(0, -1, 0), 0.025)
            final_polygons.append((n, [matrix.Transform(Gf.Vec3d(*p)) for p in points]))
    print(f"Verified {path.name}: {expected_count} white portal squares, sizes, frames and world transforms")

# A front-view inspection companion, generated from the actual USD world vertices.
svg = ['<svg xmlns="http://www.w3.org/2000/svg" width="1100" height="500" viewBox="0 0 1100 500">',
       '<rect width="1100" height="500" fill="#18212b"/>',
       '<g font-family="sans-serif" fill="#d8e2ef">',
       '<text x="50" y="45" font-size="24">Merged portal map: A, B, C</text>',
       '<text x="50" y="75" font-size="15">Front view from −Y · Z up · meters · white squares are 0.4 m wide</text>']
for x in range(5):
    px = 180 + x * 180
    svg.append(f'<path d="M {px} 115 V 415" stroke="#35404d"/><text x="{px}" y="445" text-anchor="middle">{x} m</text>')
for n, points in sorted(final_polygons):
    coords = " ".join(f"{180+p[0]*180:.3f},{490-p[2]*180:.3f}" for p in points)
    svg.append(f'<polygon points="{coords}" fill="white" stroke="#adc7df"/>')
    svg.append(f'<text x="{180+(n-1)*360}" y="300" text-anchor="middle" font-size="22">{chr(64+n)}</text>')
svg.append('</g></svg>')
(root / '05-peer1-ABC-preview.svg').write_text('\n'.join(svg))


combined = root / '06-portals-and-voxels.usda'
if combined.exists():
    stage = Usd.Stage.Open(str(combined))
    assert UsdGeom.GetStageUpAxis(stage) == UsdGeom.Tokens.z
    assert UsdGeom.GetStageMetersPerUnit(stage) == 1.0
    assert stage.GetPrimAtPath('/Map').GetAttribute('auki:rootFrameId').Get() == 'store-frame'
    assert stage.GetPrimAtPath('/Map/Voxels').GetAttribute('auki:frameId').Get() == 'store-frame'
    portals = [p for p in stage.Traverse() if p.IsA(UsdGeom.Mesh)]
    cubes = [p for p in stage.Traverse() if p.IsA(UsdGeom.Cube)]
    assert len(portals) == 3 and len(cubes) == 27
    for portal in portals:
        mesh = UsdGeom.Mesh(portal)
        assert math.isclose((mesh.GetPointsAttr().Get()[1] - mesh.GetPointsAttr().Get()[0]).GetLength(), 0.4, abs_tol=1e-6)
        assert list(mesh.GetDisplayColorAttr().Get()) == [Gf.Vec3f(1, 1, 1)]
    expected = json.loads((root / '06-expected-voxel-centers.json').read_text())
    centers = []
    bounds = UsdGeom.BBoxCache(Usd.TimeCode.Default(), [UsdGeom.Tokens.default_])
    for prim in cubes:
        cube = UsdGeom.Cube(prim)
        assert math.isclose(cube.GetSizeAttr().Get(), 0.1, abs_tol=1e-9)
        assert prim.GetAttribute('auki:occupancyEvidence').Get() > 0
        matrix = UsdGeom.Xformable(prim).ComputeLocalToWorldTransform(Usd.TimeCode.Default())
        center = matrix.Transform(Gf.Vec3d(0, 0, 0))
        assert any(Gf.IsClose(center, Gf.Vec3d(*p), 1e-6) for p in expected), center
        world_bounds = bounds.ComputeWorldBound(prim).ComputeAlignedRange()
        assert Gf.IsClose(world_bounds.GetSize(), Gf.Vec3d(0.1, 0.1, 0.1), 1e-6)
        centers.append(center)
    assert len({tuple(p) for p in centers}) == 27
    for point in expected:
        assert any(Gf.IsClose(center, Gf.Vec3d(*point), 1e-6) for center in centers)
    preview = svg[:-1]
    preview = [line.replace('Merged portal map: A, B, C', 'Portal map + occupied voxels')
               .replace('white squares are 0.4 m wide', 'white portals: 0.4 m · teal voxels: 0.1 m, 0.45 m in front')
               for line in preview]
    for center in centers:
        preview.append(f'<rect x="{180+(center[0]-0.05)*180:.3f}" y="{490-(center[2]+0.05)*180:.3f}" width="18" height="18" fill="#1fb3cc" stroke="#0d5361"/>')
    preview.append('</g></svg>')
    (root / '06-portals-and-voxels-preview.svg').write_text('\n'.join(preview))
    print('Verified combined USDA: 3 white 40cm portals + 27 occupied 10cm cubes; every world center matches independent fixture geometry')
