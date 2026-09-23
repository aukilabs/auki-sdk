# auki-navigation-py

PyO3 bindings for [`auki-navigation`](../../auki-navigation): bake / path / restrict.

Load meshes with **`auki_geometry`**. Occupancy slice is **`auki_raster`**.

```python
import auki_geometry
import auki_navigation

mesh = auki_geometry.parse_obj(open("floor.obj").read())
nav = auki_navigation.NavMesh.bake(mesh, auki_navigation.BakeProfile.dsc(0.4))
path = nav.find_path([{"x": -4, "y": 0, "z": -4}, {"x": 4, "y": 0, "z": 4}])
```

Coordinates: metres, **Y-up**. Radius is metres.
