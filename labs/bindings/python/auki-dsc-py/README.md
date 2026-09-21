# auki-dsc-py

PyO3 bindings for [`auki-dsc`](../../auki-dsc): local Recast bake + landmass path/restrict + occlusion.

```bash
# from auki-sdk root
maturin develop -m labs/bindings/python/auki-dsc-py/Cargo.toml
python -m pytest labs/bindings/python/auki-dsc-py/python_tests
```

```python
import auki_dsc

mesh = auki_dsc.parse_obj(open("floor.obj").read())
# Real agent radius in metres (ceil → voxels).
nav = auki_dsc.NavMesh.bake(mesh, auki_dsc.BakeProfile.dsc(0.4))
# DSC HTTP radius:0.05 erodes 0 voxels (int trunc). For bit-match use dsc(0.0).
path = nav.find_path([{"x": -4, "y": 0, "z": -4}, {"x": 4, "y": 0, "z": 4}])
print(path["total_distance"], len(path["full"]))
```

Coordinates: metres, **Y-up**.
