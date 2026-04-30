# Parking lot — auki-ros-adapter

---

## `r2r` typesupport blocker

`r2r` 0.9.5's compile-time-generated `sensor_msgs` typesupport doesn't match the CDR layout of the realsense camera driver's published messages. Real ROS2 subscription fails at runtime on the K1. Path forward options:

- **(a)** Wait for upstream `r2r` fix.
- **(b)** Switch to a ROS2 client library that uses introspection-based deserialization (e.g. `rclrs`).
- **(c)** Build PyO3 bindings around the SDK and let consumers use `rclpy` (which works) — `boosterapp` is already running a pure-Python re-implementation as a workaround, so this would formalize that pattern.
- **(d)** Re-implement the DDS subscription path in pure Rust without `r2r`.

Each has different effort, timeline, and drift-risk tradeoffs; (c) preserves the Python sidecar drift risk; (d) is the most work but most controlled. No decision yet.
