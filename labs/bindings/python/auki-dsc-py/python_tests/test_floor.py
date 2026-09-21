"""Smoke tests for auki_dsc (floor.obj bake + path)."""

from __future__ import annotations

from pathlib import Path

import auki_dsc
import pytest

FIXTURES = Path(__file__).resolve().parents[4] / "auki-dsc" / "fixtures"


@pytest.fixture(scope="module")
def floor_nav():
    text = (FIXTURES / "floor.obj").read_text()
    mesh = auki_dsc.parse_obj(text)
    assert mesh.vertex_count == 4
    assert mesh.triangle_count == 2
    nav = auki_dsc.NavMesh.bake(mesh, auki_dsc.BakeProfile.dsc(0.05))
    return nav


def test_find_path_across_floor(floor_nav):
    path = floor_nav.find_path(
        [{"x": -4.0, "y": 0.0, "z": -4.0}, {"x": 4.0, "y": 0.0, "z": 4.0}]
    )
    assert len(path["full"]) >= 2
    assert path["total_distance"] > 0.0
    assert path["full"][0]["x"] == pytest.approx(-4.0, abs=0.2)
    assert path["full"][-1]["x"] == pytest.approx(4.0, abs=0.2)


def test_restrict_returns_on_mesh(floor_nav):
    r = floor_nav.restrict({"x": 0.0, "y": 5.0, "z": 0.0}, 0.5)
    assert "restricted" in r
    assert abs(r["restricted"]["x"]) < 0.5
    assert abs(r["restricted"]["z"]) < 0.5
    # Snapped onto the floor plane (y near 0), not left at y=5.
    assert abs(r["restricted"]["y"]) < 1.0


def test_occlusion_cross_section():
    text = (FIXTURES / "floor.obj").read_text()
    mesh = auki_dsc.parse_obj(text)
    occ = auki_dsc.OcclusionMesh.from_mesh(mesh)
    cs = occ.cross_section(0.0, 10.0)
    assert cs["width"] > 0
    assert cs["height"] > 0
    assert cs["png"][:8] == b"\x89PNG\r\n\x1a\n"
    assert "resolution" in cs["map_yaml"]


def test_dsc_radius_preset():
    assert auki_dsc.BakeProfile.dsc(0.0).walkable_radius == 0.0
    assert auki_dsc.BakeProfile.dsc(0.4).walkable_radius == pytest.approx(0.4)
    assert auki_dsc.BakeProfile.robot_r40().walkable_radius == pytest.approx(0.4)
