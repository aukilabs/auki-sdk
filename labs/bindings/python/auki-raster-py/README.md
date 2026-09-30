# auki-raster-py

```python
import auki_geometry
import auki_raster

mesh = auki_geometry.parse_obj(obj_text)
cs = auki_raster.cross_section(mesh, 0.1, 20.0)
# cs["png"], cs["map_yaml"], cs["width"], …
```
