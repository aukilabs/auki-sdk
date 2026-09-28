#![cfg(feature = "components")]
use auki_components::*;
use auki_scenegraph::{component::*, directory::*, *};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
fn context() -> InvocationContext {
    InvocationContext {
        invocation_id: "test".into(),
        caller_peer_id: "owner".into(),
        caller_component_id: "admin".into(),
    }
}
fn definition(id: &str, name: &str, domain: &str) -> MapDefinition {
    MapDefinition {
        map_id: id.into(),
        name: Some(name.into()),
        domain_reference: Some(domain.into()),
        frame: MapFrame::z_up_meters(format!("{id}-frame"), "chosen origin"),
    }
}
fn map(runtime: &ComponentRuntime, id: &str, name: &str) -> MapComponent {
    MapComponent::new(
        runtime,
        MapComponentConfig {
            component_id: id.into(),
            publication_id: id.into(),
            clock_id: "clock".into(),
            map: definition(id, name, "store"),
        },
        || 1,
        |_| true,
        |_| true,
    )
    .unwrap()
}
fn directory(runtime: &ComponentRuntime, clock: Arc<AtomicU64>) -> MapDirectoryComponent {
    MapDirectoryComponent::new(
        runtime,
        MapDirectoryConfig {
            component_id: "directory".into(),
            publication_id: "directory-run".into(),
            clock_id: "clock".into(),
            domain_reference: "store".into(),
        },
        move || clock.fetch_add(1, Ordering::SeqCst),
        |c| c.caller_peer_id == "owner",
        |c| c.caller_peer_id == "owner" && c.caller_component_id == "admin",
    )
    .unwrap()
}
fn update(
    dir: &MapDirectoryComponent,
    change: DirectoryChange,
) -> Result<SnapshotReference, InvocationError> {
    InMemoryTransport
        .invoke(
            dir.update_directory(),
            context(),
            UpdateMapDirectory {
                expected_snapshot: dir.snapshot_reference(),
                change,
            },
        )
        .map(|v| v.result)
}
fn register(dir: &MapDirectoryComponent, map: &MapComponent) {
    update(
        dir,
        DirectoryChange::Register {
            entry: DirectoryMap::from_product(&map.product()).unwrap(),
        },
    )
    .unwrap();
}
fn resolve(
    dir: &MapDirectoryComponent,
    selector: MapSelector,
) -> Result<ResolvedMap, InvocationError> {
    InMemoryTransport
        .invoke(
            dir.resolve_map(),
            context(),
            ResolveMap {
                domain_reference: "store".into(),
                selector,
            },
        )
        .map(|v| v.result)
}

#[test]
fn domain_only_resolves_explicit_default_and_named_maps_remain_available() {
    let runtime = ComponentRuntime::new("owner");
    let dir = directory(&runtime, Arc::new(AtomicU64::new(1)));
    let a = map(&runtime, "a", "Ground floor");
    let b = map(&runtime, "b", "Stockroom");
    let a_snapshot = a.snapshot_reference();
    let b_snapshot = b.snapshot_reference();
    assert!(resolve(&dir, MapSelector::Default).is_err());
    register(&dir, &a);
    register(&dir, &b);
    assert!(resolve(&dir, MapSelector::Default).is_err());
    update(
        &dir,
        DirectoryChange::SetDefault {
            map_id: Some("a".into()),
        },
    )
    .unwrap();
    let domain_only: ResolveMap = serde_json::from_str(r#"{"domain_reference":"store"}"#).unwrap();
    let selected = InMemoryTransport
        .invoke(dir.resolve_map(), context(), domain_only)
        .unwrap()
        .result;
    assert_eq!(selected.selected.product, a.product().reference());
    assert_eq!(
        resolve(&dir, MapSelector::Name("Stockroom".into()))
            .unwrap()
            .selected
            .product,
        b.product().reference()
    );
    assert!(resolve(&dir, MapSelector::Name("stockroom".into())).is_err());
    let before = runtime.catalog().revision();
    let identity = dir.product().reference();
    update(
        &dir,
        DirectoryChange::SetDefault {
            map_id: Some("b".into()),
        },
    )
    .unwrap();
    assert_eq!(
        resolve(&dir, MapSelector::Default)
            .unwrap()
            .selected
            .product,
        b.product().reference()
    );
    assert_eq!(
        resolve(&dir, MapSelector::Id("a".into()))
            .unwrap()
            .selected
            .product,
        a.product().reference()
    );
    assert_eq!(a.snapshot_reference(), a_snapshot);
    assert_eq!(b.snapshot_reference(), b_snapshot);
    assert_eq!(dir.product().reference(), identity);
    let metadata = runtime
        .catalog()
        .product(&identity.product_id)
        .unwrap()
        .metadata
        .unwrap();
    assert_eq!(metadata.source_sequence, dir.snapshot_reference().sequence);
    assert_eq!(metadata.schema, MAP_DIRECTORY_SCHEMA);
    let advertised: DomainMapDirectory = serde_json::from_value(metadata.value).unwrap();
    assert_eq!(advertised.default_map_id.as_deref(), Some("b"));
    assert_eq!(advertised.maps.len(), 2);
    assert!(runtime.catalog().revision() > before);
    let revision = runtime.catalog().revision();
    update(
        &dir,
        DirectoryChange::SetDefault {
            map_id: Some("b".into()),
        },
    )
    .unwrap();
    assert_eq!(runtime.catalog().revision(), revision);
}

#[test]
fn wrong_domain_duplicate_names_and_unknown_default_are_rejected() {
    let runtime = ComponentRuntime::new("owner");
    let dir = directory(&runtime, Arc::new(AtomicU64::new(1)));
    let a = map(&runtime, "a", "Main");
    register(&dir, &a);
    let before = dir.snapshot_reference();
    let mut duplicate = DirectoryMap::from_product(&a.product()).unwrap();
    duplicate.map.map_id = "b".into();
    assert!(update(&dir, DirectoryChange::Register { entry: duplicate }).is_err());
    let mut foreign = DirectoryMap::from_product(&a.product()).unwrap();
    foreign.map.domain_reference = Some("elsewhere".into());
    assert!(update(&dir, DirectoryChange::Register { entry: foreign }).is_err());
    assert!(
        update(
            &dir,
            DirectoryChange::SetDefault {
                map_id: Some("missing".into())
            }
        )
        .is_err()
    );
    assert!(
        InMemoryTransport
            .invoke(
                dir.resolve_map(),
                context(),
                ResolveMap {
                    domain_reference: "elsewhere".into(),
                    selector: MapSelector::Id("a".into())
                }
            )
            .is_err()
    );
    assert_eq!(dir.snapshot_reference(), before);
}
#[test]
fn removing_default_clears_it_without_choosing_another_map() {
    let runtime = ComponentRuntime::new("owner");
    let dir = directory(&runtime, Arc::new(AtomicU64::new(1)));
    let a = map(&runtime, "a", "Main");
    let b = map(&runtime, "b", "Other");
    register(&dir, &a);
    register(&dir, &b);
    update(
        &dir,
        DirectoryChange::SetDefault {
            map_id: Some("a".into()),
        },
    )
    .unwrap();
    update(&dir, DirectoryChange::Remove { map_id: "a".into() }).unwrap();
    assert!(resolve(&dir, MapSelector::Default).is_err());
    assert!(resolve(&dir, MapSelector::Name("Main".into())).is_err());
    assert!(resolve(&dir, MapSelector::Name("Other".into())).is_ok());
}
#[test]
fn authorization_and_compare_and_set_protect_default_selection() {
    let runtime = ComponentRuntime::new("owner");
    let dir = directory(&runtime, Arc::new(AtomicU64::new(1)));
    let a = map(&runtime, "a", "Main");
    register(&dir, &a);
    let request = UpdateMapDirectory {
        expected_snapshot: dir.snapshot_reference(),
        change: DirectoryChange::SetDefault {
            map_id: Some("a".into()),
        },
    };
    let mut read_only = context();
    read_only.caller_component_id = "reader".into();
    assert!(matches!(
        InMemoryTransport.invoke(dir.update_directory(), read_only, request.clone()),
        Err(InvocationError::Unauthorized)
    ));
    let mut stranger = context();
    stranger.caller_peer_id = "stranger".into();
    assert!(matches!(
        InMemoryTransport.invoke(
            dir.resolve_map(),
            stranger,
            ResolveMap {
                domain_reference: "store".into(),
                selector: MapSelector::Id("a".into())
            }
        ),
        Err(InvocationError::Unauthorized)
    ));
    InMemoryTransport
        .invoke(dir.update_directory(), context(), request.clone())
        .unwrap();
    assert!(
        InMemoryTransport
            .invoke(dir.update_directory(), context(), request)
            .is_err()
    );
}
#[test]
fn concurrent_defaults_cannot_both_commit_against_the_same_directory() {
    let runtime = ComponentRuntime::new("owner");
    let dir = Arc::new(directory(&runtime, Arc::new(AtomicU64::new(1))));
    let a = map(&runtime, "a", "Main");
    let b = map(&runtime, "b", "Other");
    register(&dir, &a);
    register(&dir, &b);
    let base = dir.snapshot_reference();
    let handles: [_; 2] = ["a", "b"].map(|id| {
        let dir = dir.clone();
        let base = base.clone();
        std::thread::spawn(move || {
            InMemoryTransport
                .invoke(
                    dir.update_directory(),
                    context(),
                    UpdateMapDirectory {
                        expected_snapshot: base,
                        change: DirectoryChange::SetDefault {
                            map_id: Some(id.into()),
                        },
                    },
                )
                .is_ok()
        })
    });
    assert_eq!(
        handles
            .into_iter()
            .map(|h| usize::from(h.join().unwrap()))
            .sum::<usize>(),
        1
    );
}
#[test]
fn failed_retention_clock_regression_and_shutdown_do_not_publish_new_defaults() {
    let runtime = ComponentRuntime::new("owner");
    let clock = Arc::new(AtomicU64::new(1));
    let dir = directory(&runtime, clock.clone());
    let a = map(&runtime, "a", "Main");
    register(&dir, &a);
    let before = dir.snapshot_reference();
    clock.store(1, Ordering::SeqCst);
    assert!(
        update(
            &dir,
            DirectoryChange::SetDefault {
                map_id: Some("a".into())
            }
        )
        .is_err()
    );
    assert_eq!(dir.snapshot_reference(), before);
    clock.store(10, Ordering::SeqCst);
    dir.product().buffer().close();
    assert!(
        update(
            &dir,
            DirectoryChange::SetDefault {
                map_id: Some("a".into())
            }
        )
        .is_err()
    );
    assert_eq!(dir.snapshot_reference(), before);
    assert!(resolve(&dir, MapSelector::Id("a".into())).is_err());
    dir.close();
}
