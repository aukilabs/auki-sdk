//! URDF advertise helpers: parse with `quick-xml`, rewrite mesh paths, blob package.

use std::io;
use std::path::{Component, Path, PathBuf};

use crate::{
    DeviceModelBody, DeviceModelFormat, Error, MAX_BLOB_BYTES, MeshBlobRef, Result, put_blob,
    read_at_capped, validate_registry_id,
};

/// Result of [`put_urdf_package`]: rewritten URDF + mesh blobs on disk and
/// a ready-to-register [`DeviceModelBody`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutUrdfPackage {
    /// Stable id taken from the URDF `<robot name>` (lowercased), or the
    /// package directory name when the attribute is missing.
    pub device_model_id: String,
    /// Body suitable for [`crate::write_device_model`] / session `register_device_model`.
    pub body: DeviceModelBody,
}

/// Rewrite mesh `filename`s to package-relative paths, `put_blob` the URDF
/// and every referenced mesh, and return a [`DeviceModelBody`].
///
/// # Package layout
///
/// By default (`package_root = None`) the package directory is the **parent
/// of the `.urdf` file** (flattened packs: URDF beside `meshes/`). Pass
/// `package_root = Some(pkg)` for stock ROS trees where the URDF lives under
/// `pkg/urdf/` and meshes under `pkg/meshes/` — mesh resolution is fail-closed
/// under that root (no walk-up).
///
/// # Mesh attribute parsing
///
/// Uses `quick-xml` to collect `<mesh … filename="…">` / `'…'` (including
/// whitespace around `=`). Ignores comments and `mesh_filename=`.
///
/// Fails if the URDF references any mesh that cannot be resolved under the
/// package directory. Rewrite happens once at publish time so consumers do
/// not need a second path rewrite after dig.
pub fn put_urdf_package(
    app_root: &Path,
    urdf_path: &Path,
    root_convention: Option<String>,
    package_root: Option<&Path>,
) -> Result<PutUrdfPackage> {
    let urdf_bytes = read_at_capped(urdf_path, MAX_BLOB_BYTES)?.ok_or_else(|| {
        Error::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("URDF not found: {}", urdf_path.display()),
        ))
    })?;
    let urdf_text = String::from_utf8(urdf_bytes).map_err(|error| {
        Error::InvalidDeviceModel(format!("URDF is not UTF-8: {error}"))
    })?;
    let device_model_id = urdf_robot_name(&urdf_text)
        .or_else(|| {
            urdf_path
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .map(|name| name.to_ascii_lowercase())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "robot".into());
    validate_registry_id(&device_model_id)
        .map_err(|error| Error::InvalidDeviceModel(format!("invalid device_model_id: {error}")))?;

    let mesh_refs = collect_urdf_mesh_filenames(&urdf_text)?;
    let package_dir = package_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| {
            urdf_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf()
        });

    let mut rewritten = urdf_text;
    let mut meshes = Vec::new();
    let mut seen_paths = std::collections::BTreeSet::new();
    // Replace from the end so earlier offsets stay valid.
    for (start, end, original) in mesh_refs.into_iter().rev() {
        let rel = normalize_mesh_rel_path(&original);
        validate_mesh_rel_path(&rel)?;
        rewritten.replace_range(start..end, &rel);
        if !seen_paths.insert(rel.clone()) {
            continue;
        }
        let resolved = resolve_urdf_mesh(&package_dir, &original, &rel);
        let Some(resolved) = resolved else {
            return Err(Error::InvalidDeviceModel(format!(
                "URDF references mesh {original:?} but none resolved beside {}",
                package_dir.display()
            )));
        };
        let mesh_bytes = read_at_capped(&resolved, MAX_BLOB_BYTES)?.ok_or_else(|| {
            Error::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!("mesh not found: {}", resolved.display()),
            ))
        })?;
        let sha256 = put_blob(app_root, &mesh_bytes)?;
        meshes.push(MeshBlobRef {
            path: rel,
            sha256,
        });
    }
    meshes.sort_by(|a, b| a.path.cmp(&b.path));

    if urdf_has_leftover_package_mesh_filename(&rewritten)? {
        return Err(Error::InvalidDeviceModel(
            "URDF still contains package:// mesh refs after rewrite".into(),
        ));
    }

    let urdf_sha256 = put_blob(app_root, rewritten.as_bytes())?;
    let body = DeviceModelBody {
        model_id: device_model_id.clone(),
        format: DeviceModelFormat::Urdf {
            urdf_sha256,
            meshes,
        },
        root_convention,
    };
    Ok(PutUrdfPackage {
        device_model_id,
        body,
    })
}

fn urdf_robot_name(urdf: &str) -> Option<String> {
    let mut reader = quick_xml::Reader::from_str(urdf);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    loop {
        let event = match reader.read_event_into(&mut buf) {
            Ok(event) => event,
            Err(_) => return None,
        };
        match event {
            quick_xml::events::Event::Start(ref e) | quick_xml::events::Event::Empty(ref e)
                if e.local_name().as_ref() == b"robot" =>
            {
                for attr in e.attributes().flatten() {
                    if attr.key.as_ref() == b"name" {
                        let Ok(name) = attr.decode_and_unescape_value(reader.decoder()) else {
                            continue;
                        };
                        let name = name.trim().to_ascii_lowercase();
                        if !name.is_empty() {
                            return Some(name);
                        }
                    }
                }
            }
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    None
}

/// `(value_start, value_end, original_value)` for every `<mesh … filename="…">`.
///
/// Uses `quick-xml` so comments / non-mesh tags / `mesh_filename=` are ignored.
/// Byte spans point at the raw attribute value in `urdf` for in-place rewrite.
fn collect_urdf_mesh_filenames(urdf: &str) -> Result<Vec<(usize, usize, String)>> {
    let mut reader = quick_xml::Reader::from_str(urdf);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = match reader.read_event_into(&mut buf) {
            Ok(event) => event,
            Err(error) => {
                return Err(Error::InvalidDeviceModel(format!(
                    "URDF XML parse error: {error}"
                )));
            }
        };
        match event {
            quick_xml::events::Event::Start(ref e) | quick_xml::events::Event::Empty(ref e)
                if e.local_name().as_ref() == b"mesh" =>
            {
                let end = reader.buffer_position() as usize;
                let element = urdf
                    .get(start..end)
                    .ok_or_else(|| Error::InvalidDeviceModel("URDF mesh span out of range".into()))?;
                for attr in e.attributes() {
                    let attr = attr.map_err(|error| {
                        Error::InvalidDeviceModel(format!("URDF mesh attribute error: {error}"))
                    })?;
                    if attr.key.as_ref() != b"filename" {
                        continue;
                    }
                    let raw = std::str::from_utf8(attr.value.as_ref()).map_err(|error| {
                        Error::InvalidDeviceModel(format!(
                            "URDF mesh filename is not UTF-8: {error}"
                        ))
                    })?;
                    let (rel_start, rel_end) = filename_value_span(element, raw).ok_or_else(|| {
                        Error::InvalidDeviceModel(format!(
                            "URDF mesh filename value span not found for {raw:?}"
                        ))
                    })?;
                    out.push((start + rel_start, start + rel_end, raw.to_string()));
                }
            }
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

/// Locate the raw `filename` attribute value inside one serialized mesh element.
fn filename_value_span(element: &str, raw_value: &str) -> Option<(usize, usize)> {
    for quote in ['"', '\''] {
        let needle = format!("{quote}{raw_value}{quote}");
        if let Some(pos) = element.find(&needle) {
            return Some((pos + 1, pos + 1 + raw_value.len()));
        }
    }
    None
}

/// True when a live `<mesh filename>` still points at `package://…`.
fn urdf_has_leftover_package_mesh_filename(urdf: &str) -> Result<bool> {
    Ok(collect_urdf_mesh_filenames(urdf)?
        .into_iter()
        .any(|(_, _, value)| value.trim_start().starts_with("package://")))
}

/// Strip ROS `package://pkg/` so mesh paths are package-relative.
pub fn normalize_mesh_rel_path(path: &str) -> String {
    let path = path.trim().trim_start_matches('/');
    if let Some(rest) = path.strip_prefix("package://") {
        if let Some((_, relative)) = rest.split_once('/') {
            return relative.trim_start_matches('/').to_string();
        }
        return rest.to_string();
    }
    path.to_string()
}

/// Reject empty, absolute, or `..`-containing mesh relative paths.
pub fn validate_mesh_rel_path(rel: &str) -> Result<()> {
    if rel.is_empty() {
        return Err(Error::InvalidDeviceModel(
            "URDF mesh filename resolved to an empty path".into(),
        ));
    }
    let path = Path::new(rel);
    if path.is_absolute() {
        return Err(Error::InvalidDeviceModel(format!(
            "URDF mesh path must be relative, got {rel:?}"
        )));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(Error::InvalidDeviceModel(format!(
                    "URDF mesh path must not contain '..': {rel:?}"
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(Error::InvalidDeviceModel(format!(
                    "URDF mesh path must be relative, got {rel:?}"
                )));
            }
        }
    }
    Ok(())
}

fn path_has_parent_dir(path: &Path) -> bool {
    path.components().any(|c| matches!(c, Component::ParentDir))
}

/// Resolve a mesh path fail-closed: only the exact package-relative path
/// (plus explicit `package://pkg/rel` joins under `package_dir`). No
/// basename or parent-directory guesses — those can steal the wrong file.
fn resolve_urdf_mesh(package_dir: &Path, original: &str, rel: &str) -> Option<PathBuf> {
    let mut candidates = vec![package_dir.join(rel)];
    if let Some(rest) = original.strip_prefix("package://") {
        let (pkg, relative) = rest
            .split_once('/')
            .map(|(pkg, relative)| (pkg, relative))
            .unwrap_or((rest, ""));
        candidates.push(package_dir.join(rest));
        if !relative.is_empty() {
            candidates.push(package_dir.join(pkg).join(relative));
        }
    }
    candidates.into_iter().find(|path| {
        if path_has_parent_dir(path) {
            return false;
        }
        path.is_file()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MAX_BLOB_BYTES, get_blob, list_device_models};
    use std::fs;

    #[test]
    fn put_urdf_package_rejects_oversized_urdf() {
        let pkg = tempfile::tempdir().unwrap();
        let urdf_path = pkg.path().join("robot.urdf");
        {
            let f = fs::File::create(&urdf_path).unwrap();
            f.set_len(MAX_BLOB_BYTES + 1).unwrap();
        }
        let app = tempfile::tempdir().unwrap();
        assert!(matches!(
            put_urdf_package(app.path(), &urdf_path, None, None),
            Err(Error::InvalidBlob(_))
        ));
    }

    #[test]
    fn put_urdf_package_rejects_oversized_mesh() {
        let pkg = tempfile::tempdir().unwrap();
        let meshes = pkg.path().join("meshes");
        fs::create_dir_all(&meshes).unwrap();
        {
            let f = fs::File::create(meshes.join("body.stl")).unwrap();
            f.set_len(MAX_BLOB_BYTES + 1).unwrap();
        }
        let urdf_path = pkg.path().join("robot.urdf");
        fs::write(
            &urdf_path,
            r#"<robot name="K1">
                <link name="base">
                  <visual><geometry><mesh filename="meshes/body.stl"/></geometry></visual>
                </link>
            </robot>"#,
        )
        .unwrap();
        let app = tempfile::tempdir().unwrap();
        assert!(matches!(
            put_urdf_package(app.path(), &urdf_path, None, None),
            Err(Error::InvalidBlob(_))
        ));
    }

    #[test]
    fn put_urdf_package_rejects_parent_dir_mesh() {
        let pkg = tempfile::tempdir().unwrap();
        let urdf_path = pkg.path().join("robot.urdf");
        fs::write(
            &urdf_path,
            r#"<robot name="evil">
                <link name="base">
                  <visual><geometry><mesh filename="../etc/passwd"/></geometry></visual>
                </link>
            </robot>"#,
        )
        .unwrap();
        let app = tempfile::tempdir().unwrap();
        assert!(matches!(
            put_urdf_package(app.path(), &urdf_path, None, None),
            Err(Error::InvalidDeviceModel(_))
        ));
    }

    #[test]
    fn put_urdf_package_does_not_steal_basename() {
        let pkg = tempfile::tempdir().unwrap();
        let meshes = pkg.path().join("meshes");
        fs::create_dir_all(&meshes).unwrap();
        fs::write(meshes.join("body.stl"), b"collision").unwrap();
        let urdf_path = pkg.path().join("robot.urdf");
        fs::write(
            &urdf_path,
            r#"<robot name="K1">
                <link name="base">
                  <visual><geometry><mesh filename="meshes/visual/body.stl"/></geometry></visual>
                </link>
            </robot>"#,
        )
        .unwrap();
        let app = tempfile::tempdir().unwrap();
        assert!(matches!(
            put_urdf_package(app.path(), &urdf_path, None, None),
            Err(Error::InvalidDeviceModel(_))
        ));
        assert!(list_device_models(app.path(), "unused").unwrap().is_empty());
        let blobs = app.path().join("blobs");
        assert!(!blobs.exists() || fs::read_dir(&blobs).unwrap().next().is_none());
    }

    #[test]
    fn collect_urdf_mesh_filenames_only_mesh_tags() {
        let urdf = r#"<robot name="K1">
            <!-- filename="comment.stl" -->
            <link name="base" mesh_filename="attr.stl">
              <visual><geometry><mesh filename="meshes/body.stl"/></geometry></visual>
            </link>
        </robot>"#;
        let refs = collect_urdf_mesh_filenames(urdf).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].2, "meshes/body.stl");
    }

    #[test]
    fn collect_urdf_mesh_filenames_skips_commented_mesh() {
        let urdf = r#"<robot name="K1">
            <!-- <mesh filename="package://dead/meshes/skip.stl"/> -->
            <link name="base">
              <visual><geometry><mesh filename="meshes/body.stl"/></geometry></visual>
            </link>
        </robot>"#;
        let refs = collect_urdf_mesh_filenames(urdf).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].2, "meshes/body.stl");
        assert!(!urdf_has_leftover_package_mesh_filename(urdf).unwrap());
    }

    #[test]
    fn put_urdf_package_ignores_commented_package_mesh() {
        let pkg = tempfile::tempdir().unwrap();
        let meshes = pkg.path().join("meshes");
        fs::create_dir_all(&meshes).unwrap();
        fs::write(meshes.join("body.stl"), b"mesh").unwrap();
        let urdf_path = pkg.path().join("robot.urdf");
        fs::write(
            &urdf_path,
            r#"<robot name="K1">
                <!-- <mesh filename="package://K1_URDF_Serial/meshes/gone.stl"/> -->
                <link name="base">
                  <visual><geometry><mesh filename="meshes/body.stl"/></geometry></visual>
                </link>
            </robot>"#,
        )
        .unwrap();
        let app = tempfile::tempdir().unwrap();
        let package = put_urdf_package(app.path(), &urdf_path, None, None).unwrap();
        let (_, mesh_refs) = package.body.as_urdf().unwrap();
        assert_eq!(mesh_refs.len(), 1);
        assert_eq!(mesh_refs[0].path, "meshes/body.stl");
    }

    #[test]
    fn collect_urdf_mesh_filenames_accepts_spaced_equals() {
        let urdf = r#"<robot name="K1">
            <link name="base" mesh_filename = "attr.stl">
              <visual><geometry><mesh filename = "meshes/body.stl"/></geometry></visual>
            </link>
        </robot>"#;
        let refs = collect_urdf_mesh_filenames(urdf).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].2, "meshes/body.stl");
    }

    #[test]
    fn urdf_robot_name_accepts_spaced_equals() {
        assert_eq!(
            urdf_robot_name(r#"<robot name = "K1">"#).as_deref(),
            Some("k1")
        );
        assert_eq!(
            urdf_robot_name(r#"<robot name='G1'>"#).as_deref(),
            Some("g1")
        );
    }

    #[test]
    fn urdf_robot_name_skips_commented_robot() {
        let urdf = r#"
            <!-- <robot name="OldK1"> -->
            <robot name="K1">
              <link name="base"/>
            </robot>
        "#;
        assert_eq!(urdf_robot_name(urdf).as_deref(), Some("k1"));
    }

    #[test]
    fn put_urdf_package_uses_spaced_robot_name() {
        let pkg = tempfile::tempdir().unwrap();
        let meshes = pkg.path().join("meshes");
        fs::create_dir_all(&meshes).unwrap();
        fs::write(meshes.join("body.stl"), b"mesh").unwrap();
        let urdf_path = pkg.path().join("robot.urdf");
        fs::write(
            &urdf_path,
            r#"<robot name = "K1">
                <link name="base">
                  <visual><geometry><mesh filename="meshes/body.stl"/></geometry></visual>
                </link>
            </robot>"#,
        )
        .unwrap();
        let app = tempfile::tempdir().unwrap();
        let package = put_urdf_package(app.path(), &urdf_path, None, None).unwrap();
        assert_eq!(package.device_model_id, "k1");
    }

    #[test]
    fn put_urdf_package_rewrites_and_blobs() {
        let pkg = tempfile::tempdir().unwrap();
        let meshes = pkg.path().join("meshes");
        fs::create_dir_all(&meshes).unwrap();
        fs::write(meshes.join("body.stl"), b"mesh").unwrap();
        let urdf_path = pkg.path().join("robot.urdf");
        fs::write(
            &urdf_path,
            r#"<robot name="K1">
                <link name="base">
                  <visual><geometry><mesh filename="package://K1_URDF_Serial/meshes/body.stl"/></geometry></visual>
                </link>
            </robot>"#,
        )
        .unwrap();
        let app = tempfile::tempdir().unwrap();
        let package =
            put_urdf_package(app.path(), &urdf_path, Some("ros_body".into()), None).unwrap();
        assert_eq!(package.device_model_id, "k1");
        let (urdf_sha, meshes) = package.body.as_urdf().unwrap();
        assert_eq!(meshes.len(), 1);
        assert_eq!(meshes[0].path, "meshes/body.stl");
        let urdf = get_blob(app.path(), urdf_sha).unwrap().unwrap();
        let text = String::from_utf8(urdf).unwrap();
        assert!(text.contains(r#"filename="meshes/body.stl""#));
        assert!(!text.contains("package://"));
    }

    #[test]
    fn put_urdf_package_rewrites_spaced_filename_attr() {
        let pkg = tempfile::tempdir().unwrap();
        let meshes = pkg.path().join("meshes");
        fs::create_dir_all(&meshes).unwrap();
        fs::write(meshes.join("body.stl"), b"mesh").unwrap();
        let urdf_path = pkg.path().join("robot.urdf");
        fs::write(
            &urdf_path,
            r#"<robot name="K1">
                <link name="base">
                  <visual><geometry><mesh filename = "package://K1_URDF_Serial/meshes/body.stl"/></geometry></visual>
                </link>
            </robot>"#,
        )
        .unwrap();
        let app = tempfile::tempdir().unwrap();
        let package = put_urdf_package(app.path(), &urdf_path, None, None).unwrap();
        let (urdf_sha, _) = package.body.as_urdf().unwrap();
        let text = String::from_utf8(get_blob(app.path(), urdf_sha).unwrap().unwrap()).unwrap();
        assert!(text.contains(r#"filename = "meshes/body.stl""#));
        assert!(!text.contains("package://"));
    }

    #[test]
    fn put_urdf_package_accepts_stock_ros_with_package_root() {
        let pkg = tempfile::tempdir().unwrap();
        let urdf_dir = pkg.path().join("urdf");
        let meshes = pkg.path().join("meshes");
        fs::create_dir_all(&urdf_dir).unwrap();
        fs::create_dir_all(&meshes).unwrap();
        fs::write(meshes.join("body.stl"), b"mesh").unwrap();
        let urdf_path = urdf_dir.join("robot.urdf");
        fs::write(
            &urdf_path,
            r#"<robot name="stock">
                <link name="base">
                  <visual><geometry><mesh filename="package://pkg/meshes/body.stl"/></geometry></visual>
                </link>
            </robot>"#,
        )
        .unwrap();
        let app = tempfile::tempdir().unwrap();
        assert!(matches!(
            put_urdf_package(app.path(), &urdf_path, None, None),
            Err(Error::InvalidDeviceModel(_))
        ));
        let package =
            put_urdf_package(app.path(), &urdf_path, None, Some(pkg.path())).unwrap();
        let (_, meshes) = package.body.as_urdf().unwrap();
        assert_eq!(meshes[0].path, "meshes/body.stl");
    }
}
