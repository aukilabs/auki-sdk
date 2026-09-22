import auki_geometry
import auki_raster


def test_cross_section_png():
    mesh = auki_geometry.TriangleMesh.from_indexed(
        [
            {"x": -1.0, "y": 0.0, "z": -1.0},
            {"x": 1.0, "y": 0.0, "z": -1.0},
            {"x": 1.0, "y": 0.0, "z": 1.0},
            {"x": -1.0, "y": 0.0, "z": 1.0},
        ],
        [[0, 1, 2], [0, 2, 3]],
    )
    cs = auki_raster.cross_section(mesh, 0.0, 20.0)
    assert cs["png"][:8] == b"\x89PNG\r\n\x1a\n"
    assert "resolution" in cs["map_yaml"]
    assert cs["width"] >= 1
