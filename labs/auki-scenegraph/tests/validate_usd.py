"""Validate the qr_map_usda example with the actual OpenUSD parser (usd-core).

Usage: python tests/validate_usd.py /tmp/qr-map.usda
This is an optional local interoperability check, not a Rust runtime dependency.
"""
import sys
from pxr import Gf, Usd, UsdGeom

stage = Usd.Stage.Open(sys.argv[1])
assert stage
assert stage.GetDefaultPrim().GetPath().pathString == "/Map"
assert UsdGeom.GetStageUpAxis(stage) == UsdGeom.Tokens.z
assert UsdGeom.GetStageMetersPerUnit(stage) == 1.0
root = stage.GetPrimAtPath("/Map")
assert root.GetAttribute("auki:mapId").Get() == "example-qr-map"
anchors = [p for p in stage.Traverse() if p.GetAttribute("auki:kind").Get() == "qr_anchor"]
assert len(anchors) == 1
anchor = anchors[0]
assert anchor.GetAttribute("auki:anchorId").Get() == "qr-123"
assert anchor.GetAttribute("auki:qr:sideLengthMeters").Get() == 0.2
matrix = UsdGeom.Xformable(anchor).ComputeLocalToWorldTransform(Usd.TimeCode.Default())
assert Gf.IsClose(matrix.Transform(Gf.Vec3d(0, 0, 0)), Gf.Vec3d(1, 2, 3), 1e-10)
# Quaternion (0.5,0.5,0.5,0.5) rotates local +X into map +Y.
assert Gf.IsClose(matrix.Transform(Gf.Vec3d(1, 0, 0)), Gf.Vec3d(1, 3, 3), 1e-10)
print("OpenUSD verified map identity, Z-up/meters, anchor size, translation and rotation")
