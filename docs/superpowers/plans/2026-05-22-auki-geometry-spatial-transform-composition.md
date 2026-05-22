# Auki Geometry Spatial Transform Composition Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add pure `SpatialTransform` inverse, composition, and relative-transform helpers to `auki-geometry` so callers can derive `B->C` from known `A->B` and `A->C` transforms.

**Architecture:** Keep this as an IO-free math addition inside `crates/auki-geometry/src/lib.rs`. Treat `SpatialTransform` as mapping a point by `p_to = R_from_to * p_from + t_from_to`; missing translation means zero, missing orientation means identity, and output transforms are explicit normalized `Some(Vec3)` plus `Some(Quat)` values. Do not start pose-log path-finding, interpolation, graph traversal, or full `convert_pose`.

**Tech Stack:** Rust, `auki_datatypes::pose::{SpatialTransform, Vec3, Quat}`, existing `auki-geometry` quaternion/matrix helpers, Cargo unit tests.

**GitHub Task:** [#184](https://github.com/aukilabs/auki-sdk/issues/184)

---

## File Structure

- `crates/auki-geometry/src/lib.rs` - add the public transform helpers, private vector/quaternion utilities, and unit tests.

No changelog files are updated in this checkout because root changelog files are absent and `docs/superpowers/*/changelog.md` files currently contain only `deprecated`.

---

### Task 1: Add Failing Tests For Spatial Transform Math

**Files:**
- Modify: `crates/auki-geometry/src/lib.rs`
- Test: `crates/auki-geometry/src/lib.rs`

- [ ] **Step 1: Add test helpers**

Inside the existing `#[cfg(test)] mod tests`, after `assert_quat_equivalent`, add helper functions for building transforms and applying them to points:

```rust
fn assert_transform_close(actual: SpatialTransform, expected: SpatialTransform) {
    assert_vec3_close(actual.translation.unwrap(), expected.translation.unwrap());
    assert_quat_equivalent(actual.orientation.unwrap(), expected.orientation.unwrap());
}

fn vec3(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

fn quat_z_90() -> Quat {
    let half = std::f64::consts::FRAC_1_SQRT_2;
    Quat {
        x: 0.0,
        y: 0.0,
        z: half,
        w: half,
    }
}

fn quat_x_90() -> Quat {
    let half = std::f64::consts::FRAC_1_SQRT_2;
    Quat {
        x: half,
        y: 0.0,
        z: 0.0,
        w: half,
    }
}

fn transform(translation: Vec3, orientation: Quat) -> SpatialTransform {
    SpatialTransform {
        translation: Some(translation),
        orientation: Some(orientation),
    }
}

fn identity_transform() -> SpatialTransform {
    transform(
        vec3(0.0, 0.0, 0.0),
        Quat {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        },
    )
}

fn apply_transform_to_point(transform: &SpatialTransform, point: Vec3) -> Vec3 {
    let rotation = quat_to_matrix(transform.orientation.clone().unwrap());
    let rotated = apply_matrix3_to_vec3(rotation, point).unwrap();
    let translation = transform.translation.clone().unwrap();
    vec3(
        rotated.x + translation.x,
        rotated.y + translation.y,
        rotated.z + translation.z,
    )
}
```

- [ ] **Step 2: Add inverse tests**

Add tests that prove inverse handles identity, translation, and rotation:

```rust
#[test]
fn inverse_spatial_transform_inverts_identity() {
    let inverse = inverse_spatial_transform(&identity_transform()).unwrap();
    assert_transform_close(inverse, identity_transform());
}

#[test]
fn inverse_spatial_transform_round_trips_point() {
    let camera_to_slam = transform(vec3(1.0, 2.0, 3.0), quat_z_90());
    let slam_to_camera = inverse_spatial_transform(&camera_to_slam).unwrap();

    let camera_point = vec3(4.0, -1.0, 2.0);
    let slam_point = apply_transform_to_point(&camera_to_slam, camera_point.clone());
    let round_tripped = apply_transform_to_point(&slam_to_camera, slam_point);

    assert_vec3_close(round_tripped, camera_point);
}
```

- [ ] **Step 3: Add composition tests**

Add tests that pin the order: `compose_spatial_transforms(A->B, B->C) = A->C`.

```rust
#[test]
fn compose_spatial_transforms_maps_source_to_final_target() {
    let a_to_b = transform(vec3(1.0, 0.0, 0.0), quat_z_90());
    let b_to_c = transform(vec3(0.0, 2.0, 0.0), quat_x_90());

    let a_to_c = compose_spatial_transforms(&a_to_b, &b_to_c).unwrap();

    let point_in_a = vec3(3.0, 4.0, 5.0);
    let via_b = apply_transform_to_point(&b_to_c, apply_transform_to_point(&a_to_b, point_in_a.clone()));
    let direct = apply_transform_to_point(&a_to_c, point_in_a);

    assert_vec3_close(direct, via_b);
}

#[test]
fn compose_spatial_transforms_with_inverse_returns_identity() {
    let a_to_b = transform(vec3(1.0, 2.0, 3.0), quat_z_90());
    let b_to_a = inverse_spatial_transform(&a_to_b).unwrap();
    let composed = compose_spatial_transforms(&a_to_b, &b_to_a).unwrap();

    assert_transform_close(composed, identity_transform());
}
```

- [ ] **Step 4: Add relative transform tests**

Add tests for the user-facing camera/domain/SLAM case:

```rust
#[test]
fn relative_spatial_transform_derives_target_to_target_from_common_source() {
    let camera_to_slam = transform(vec3(1.0, 0.0, 0.0), quat_z_90());
    let slam_to_domain = transform(vec3(0.0, 5.0, 0.0), quat_x_90());
    let camera_to_domain = compose_spatial_transforms(&camera_to_slam, &slam_to_domain).unwrap();

    let derived_slam_to_domain =
        relative_spatial_transform(&camera_to_slam, &camera_to_domain).unwrap();

    assert_transform_close(derived_slam_to_domain, slam_to_domain);
}
```

- [ ] **Step 5: Add optional-field and error tests**

Add tests that pin default semantics and zero-quaternion rejection:

```rust
#[test]
fn spatial_transform_helpers_treat_missing_parts_as_identity_components() {
    let only_translation = SpatialTransform {
        translation: Some(vec3(2.0, 3.0, 4.0)),
        orientation: None,
    };
    let only_rotation = SpatialTransform {
        translation: None,
        orientation: Some(quat_z_90()),
    };

    let composed = compose_spatial_transforms(&only_translation, &only_rotation).unwrap();

    assert_vec3_close(composed.translation.unwrap(), vec3(-3.0, 2.0, 4.0));
    assert_quat_equivalent(composed.orientation.unwrap(), quat_z_90());
}

#[test]
fn spatial_transform_helpers_reject_zero_quaternion() {
    let bad = SpatialTransform {
        translation: None,
        orientation: Some(Quat {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 0.0,
        }),
    };

    assert!(matches!(
        inverse_spatial_transform(&bad),
        Err(GeometryError::ZeroQuaternion)
    ));
    assert!(matches!(
        compose_spatial_transforms(&bad, &identity_transform()),
        Err(GeometryError::ZeroQuaternion)
    ));
}
```

- [ ] **Step 6: Run tests to verify the red state**

Run:

```bash
cargo test -p auki-geometry
```

Expected: FAIL with unresolved functions `inverse_spatial_transform`, `compose_spatial_transforms`, and `relative_spatial_transform`.

---

### Task 2: Implement Public Spatial Transform Helpers

**Files:**
- Modify: `crates/auki-geometry/src/lib.rs`
- Test: `crates/auki-geometry/src/lib.rs`

- [ ] **Step 1: Add public functions**

After `convert_pose_convention`, add:

```rust
/// Invert a transform from `from` into `to`, returning the transform from
/// `to` back into `from`.
///
/// A missing translation is treated as zero. A missing orientation is
/// treated as identity. The returned transform stores explicit translation
/// and orientation values.
pub fn inverse_spatial_transform(transform: &SpatialTransform) -> Result<SpatialTransform> {
    let rotation = spatial_transform_rotation(transform)?;
    let inverse_rotation = transpose3(rotation);
    let translation = spatial_transform_translation(transform);
    let inverse_translation = negate_vec3(apply_matrix3_to_vec3(inverse_rotation, translation)?);

    Ok(SpatialTransform {
        translation: Some(inverse_translation),
        orientation: Some(matrix_to_quat(inverse_rotation)?),
    })
}

/// Compose `from->mid` with `mid->to`, returning `from->to`.
///
/// The transform contract is `p_to = R_from_to * p_from + t_from_to`.
pub fn compose_spatial_transforms(
    from_to_mid: &SpatialTransform,
    mid_to_to: &SpatialTransform,
) -> Result<SpatialTransform> {
    let first_rotation = spatial_transform_rotation(from_to_mid)?;
    let second_rotation = spatial_transform_rotation(mid_to_to)?;
    let first_translation = spatial_transform_translation(from_to_mid);
    let second_translation = spatial_transform_translation(mid_to_to);

    let rotation = mul3(second_rotation, first_rotation);
    let rotated_translation = apply_matrix3_to_vec3(second_rotation, first_translation)?;
    let translation = add_vec3(rotated_translation, second_translation);

    Ok(SpatialTransform {
        translation: Some(translation),
        orientation: Some(matrix_to_quat(rotation)?),
    })
}

/// Given `common->from` and `common->to`, derive `from->to`.
pub fn relative_spatial_transform(
    common_to_from: &SpatialTransform,
    common_to_to: &SpatialTransform,
) -> Result<SpatialTransform> {
    let from_to_common = inverse_spatial_transform(common_to_from)?;
    compose_spatial_transforms(&from_to_common, common_to_to)
}
```

- [ ] **Step 2: Add private vector and transform helpers**

Near the existing private helpers, add:

```rust
fn spatial_transform_translation(transform: &SpatialTransform) -> Vec3 {
    transform.translation.clone().unwrap_or(Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    })
}

fn spatial_transform_rotation(transform: &SpatialTransform) -> Result<Matrix3> {
    let orientation = transform.orientation.clone().unwrap_or(Quat {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    });
    Ok(quat_to_matrix(normalize_quat(orientation)?))
}

fn add_vec3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

fn negate_vec3(v: Vec3) -> Vec3 {
    Vec3 {
        x: -v.x,
        y: -v.y,
        z: -v.z,
    }
}
```

- [ ] **Step 3: Run focused tests**

Run:

```bash
cargo test -p auki-geometry spatial_transform -- --nocapture
```

Expected: PASS for all new tests whose names contain `spatial_transform`.

- [ ] **Step 4: Run the full crate tests**

Run:

```bash
cargo test -p auki-geometry
```

Expected: PASS for all `auki-geometry` unit tests.

- [ ] **Step 5: Commit**

Only commit after confirming the active GitHub card and branch. Use a branch named from the issue number, for example:

```bash
git add crates/auki-geometry/src/lib.rs
git commit -m "feat: add spatial transform composition helpers"
```

---

### Task 3: Sprint Documentation Superseded

Issue [#185](https://github.com/aukilabs/auki-sdk/issues/185) removes `sprint.md` files from the repository. Do not update `crates/auki-geometry/src/sprint.md` as part of this feature branch.

---

## Self-Review

- Spec coverage: The plan implements the approved small pure-math slice only: inverse, composition, and relative transform. It deliberately excludes pose-log graph traversal, interpolation, path-finding, and full `convert_pose`.
- Placeholder scan: No `TBD`, `TODO`, or open-ended implementation placeholders remain.
- Type consistency: All public APIs use `auki_datatypes::pose::SpatialTransform`; helper math uses the existing `Matrix3`, `Vec3`, and `Quat` types in `auki-geometry`.

## Execution Options

1. **Subagent-Driven (recommended)** - Dispatch a fresh subagent per task, review between tasks, and keep edits isolated.
2. **Inline Execution** - Execute tasks in this session using `superpowers:executing-plans`, with checkpoints after each task.
