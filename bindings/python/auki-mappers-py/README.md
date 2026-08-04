# auki-mappers-py

Python access to SDK-native Mappers. The initial `Voxelizer` accepts a point
cloud payload only after it has entered the SDK, together with the exact
Rangefinder registry entry and pose selected by the application from SDK
resources. It returns the protobuf bytes for a mergeable `MapUpdate`.

```python
voxelizer = auki_mappers.Voxelizer(voxel_size_m=0.05, chunk_dimension=16)
update = voxelizer.map_point_cloud(
    point_cloud_data,
    sensor_registry_entry,
    [tx, ty, tz, qx, qy, qz, qw],
    free_delta=-0.25,
    occupied_delta=1.0,
)
```

Robot and ROS APIs are deliberately absent from this binding.

