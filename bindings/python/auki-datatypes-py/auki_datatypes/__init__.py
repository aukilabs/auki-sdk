"""Python betterproto bindings for the Auki SDK's shared cross-language data types.

The `.proto` files live in [`auki-datatypes/proto/`](../../../../crates/auki-datatypes/proto/);
this package contains betterproto-generated dataclass-shaped Python code,
one submodule per `.proto` package. Cross-language byte equality with
the Rust prost encoder is verified by locked-vector tests in
[`tests/test_locked_vectors.py`](../tests/test_locked_vectors.py).

Surface mirrors the Rust crate's module layout one-to-one — every
`auki_datatypes::<name>::<Type>` in Rust has a corresponding
`auki_datatypes.<name>.<Type>` in Python.

Re-exported submodules:
    audio, camera, detection, joint_encoders, joint_encoders_stream,
    point_cloud, point_cloud_stream, pose, stream, time_transform.

The generated files live one level down, in the `auki/` package
(matching the proto-package path). The re-exports below let consumers
write `from auki_datatypes import detection` without the double
`auki_datatypes.auki.detection` indirection.
"""

from .auki import audio
from .auki import camera
from .auki import detection
from .auki import joint_encoders
from .auki import joint_encoders_stream
from .auki import point_cloud
from .auki import point_cloud_stream
from .auki import pose
from .auki import stream
from .auki import time_transform

__all__ = [
    "audio",
    "camera",
    "detection",
    "joint_encoders",
    "joint_encoders_stream",
    "point_cloud",
    "point_cloud_stream",
    "pose",
    "stream",
    "time_transform",
]
