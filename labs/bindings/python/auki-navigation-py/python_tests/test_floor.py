"""Smoke tests for auki_navigation (floor.obj bake + path)."""

from __future__ import annotations

from pathlib import Path

import auki_geometry
import auki_navigation
import pytest

FIXTURES = Path(__file__).resolve().parents[4] / "auki-navigation" / "fixtures"


@pytest.fixture(scope="module")
def floor_nav():
    text = (FIXTURES / "floor.obj").read_text()
    mesh = auki_geometry.parse_obj(text)
    assert mesh.vertex_count == 4
    assert mesh.triangle_count == 2
    nav = auki_navigation.NavMesh.bake(mesh, auki_navigation.BakeProfile.dsc(0.05))
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
    assert abs(r["restricted"]["y"]) < 1.0


def test_from_indexed_bakes():
    mesh = auki_geometry.TriangleMesh.from_indexed(
        [
            {"x": -1.0, "y": 0.0, "z": -1.0},
            {"x": 1.0, "y": 0.0, "z": -1.0},
            {"x": 1.0, "y": 0.0, "z": 1.0},
            {"x": -1.0, "y": 0.0, "z": 1.0},
        ],
        [[0, 1, 2], [0, 2, 3]],
    )
    assert mesh.vertex_count == 4
    assert mesh.triangle_count == 2
    nav = auki_navigation.NavMesh.bake(mesh, auki_navigation.BakeProfile.dsc(0.05))
    path = nav.find_path(
        [{"x": -0.5, "y": 0.0, "z": -0.5}, {"x": 0.5, "y": 0.0, "z": 0.5}]
    )
    assert path["total_distance"] > 0.0


def test_dsc_radius_preset():
    assert auki_navigation.BakeProfile.dsc(0.0).walkable_radius == 0.0
    assert auki_navigation.BakeProfile.dsc(0.4).walkable_radius == pytest.approx(0.4)
    assert auki_navigation.BakeProfile.robot_r40().walkable_radius == pytest.approx(0.4)
