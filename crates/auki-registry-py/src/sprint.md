# Sprint — auki-registry-py

Current focus:

- Ship the Python producer path for declaring frame conventions and frame-pinned sensor registry entries.
- Keep the binding dict-oriented and thin over Rust validation.

Next:

- Add higher-level ROS/K1 convenience constructors only after Boosterapp settles its exact topic metadata shape. The generic registry surface is enough for the current stream-manifest builder flow.
- Consider `auki-ros-adapter-py` separately if Python needs ROS message translation, rather than bloating registry bindings with adapter logic.

Long-term:

- Keep Python and Rust registry hashes locked through shared fixtures whenever schema changes.
