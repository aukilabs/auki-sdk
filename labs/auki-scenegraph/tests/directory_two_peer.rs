#![cfg(all(feature = "components", not(target_arch = "wasm32")))]
mod support;
use auki_component_protocol::{
    CatalogResponse, ComponentProtocolClient, ComponentProtocolEndpoint,
};
use auki_components::*;
use auki_scenegraph::{component::*, directory::*, *};
use auki_sdk::{AukiPeer, Identity};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use uuid::Uuid;
fn make_map(runtime: &ComponentRuntime, id: &str, name: &str, domain: &str) -> MapComponent {
    MapComponent::new(
        runtime,
        MapComponentConfig {
            component_id: id.into(),
            publication_id: id.into(),
            clock: fixture_clock("clock"),
            map: MapDefinition {
                map_id: id.into(),
                name: Some(name.into()),
                domain_reference: Some(domain.into()),
                frame: MapFrame::z_up_meters(format!("{id}-frame"), "chosen origin"),
            },
        },
        || 1,
        |_| true,
        |_| false,
    )
    .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn catalog_and_remote_queries_agree_on_default_and_named_map_selection() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let domain = Uuid::new_v4();
        let sid = Identity::generate();
        let cid = Identity::generate();
        let (server, _sa) = AukiPeer::start_external(
            sid.clone(),
            support::authority(&sid, domain),
            support::direct_config(),
        )
        .await
        .unwrap();
        let (consumer, _ca) = AukiPeer::start_external(
            cid.clone(),
            support::authority(&cid, domain),
            support::direct_config(),
        )
        .await
        .unwrap();
        let route = server.listen_addresses()[0].clone();
        let runtime = ComponentRuntime::new(server.peer_id().to_string());
        let a = make_map(&runtime, "a", "Main", &domain.to_string());
        let b = make_map(&runtime, "b", "Other", &domain.to_string());
        let clock = AtomicU64::new(1);
        let writer = consumer.peer_id().to_string();
        let directory = MapDirectoryComponent::new(
            &runtime,
            MapDirectoryConfig {
                component_id: "directory".into(),
                publication_id: "directory-run".into(),
                clock: fixture_clock("clock"),
                domain_reference: domain.to_string(),
            },
            move || clock.fetch_add(1, Ordering::SeqCst),
            |_| true,
            move |c| c.caller_peer_id == writer && c.caller_component_id == "admin",
        )
        .unwrap();
        let context = InvocationContext {
            invocation_id: "seed".into(),
            caller_peer_id: consumer.peer_id().to_string(),
            caller_component_id: "admin".into(),
        };
        for map in [&a, &b] {
            InMemoryTransport
                .invoke(
                    directory.update_directory(),
                    context.clone(),
                    UpdateMapDirectory {
                        expected_snapshot: directory.snapshot_reference(),
                        change: DirectoryChange::Register {
                            entry: DirectoryMap::from_product(&map.product()).unwrap(),
                        },
                    },
                )
                .unwrap();
        }
        InMemoryTransport
            .invoke(
                directory.update_directory(),
                context,
                UpdateMapDirectory {
                    expected_snapshot: directory.snapshot_reference(),
                    change: DirectoryChange::SetDefault {
                        map_id: Some("a".into()),
                    },
                },
            )
            .unwrap();
        let endpoint = ComponentProtocolEndpoint::mount(server.protocols(), runtime).unwrap();
        endpoint.export_product(&a.product()).unwrap();
        endpoint.export_product(&b.product()).unwrap();
        endpoint.export_product(&directory.product()).unwrap();
        endpoint.export_operable(directory.resolve_map()).unwrap();
        endpoint
            .export_operable(directory.update_directory())
            .unwrap();
        let client = ComponentProtocolClient::new(consumer.protocols());
        let CatalogResponse::Snapshot { snapshot: catalog } = client
            .catalog_exact(server.peer_id(), route.clone(), None)
            .await
            .unwrap()
        else {
            panic!("catalog")
        };
        let advertised = catalog
            .products
            .iter()
            .find(|p| {
                p.metadata
                    .as_ref()
                    .is_some_and(|m| m.schema == MAP_DIRECTORY_SCHEMA)
            })
            .unwrap();
        let data: DomainMapDirectory =
            serde_json::from_value(advertised.metadata.as_ref().unwrap().value.clone()).unwrap();
        let domain_only = ResolveMap {
            domain_reference: domain.to_string(),
            selector: MapSelector::Default,
        };
        assert_eq!(
            data.resolve(&domain_only).unwrap().product,
            a.product().reference()
        );
        let target = directory.component().reference().clone();
        let selected: Invocation<ResolvedMap> = client
            .invoke_exact(
                server.peer_id(),
                route.clone(),
                target.clone(),
                RESOLVE_MAP,
                "reader",
                "default",
                Some(Duration::from_secs(2)),
                &domain_only,
            )
            .await
            .unwrap();
        assert_eq!(selected.result.selected.product, a.product().reference());
        assert_eq!(
            selected.result.directory_snapshot.sequence,
            advertised.metadata.as_ref().unwrap().source_sequence
        );
        let named: Invocation<ResolvedMap> = client
            .invoke_exact(
                server.peer_id(),
                route.clone(),
                target.clone(),
                RESOLVE_MAP,
                "reader",
                "named",
                Some(Duration::from_secs(2)),
                &ResolveMap {
                    domain_reference: domain.to_string(),
                    selector: MapSelector::Name("Other".into()),
                },
            )
            .await
            .unwrap();
        assert_eq!(named.result.selected.product, b.product().reference());
        let request = UpdateMapDirectory {
            expected_snapshot: selected.result.directory_snapshot,
            change: DirectoryChange::SetDefault {
                map_id: Some("b".into()),
            },
        };
        assert!(
            client
                .invoke_exact::<_, SnapshotReference>(
                    server.peer_id(),
                    route.clone(),
                    target.clone(),
                    UPDATE_MAP_DIRECTORY,
                    "reader",
                    "denied",
                    Some(Duration::from_secs(2)),
                    &request
                )
                .await
                .is_err()
        );
        let changed: Invocation<SnapshotReference> = client
            .invoke_exact(
                server.peer_id(),
                route.clone(),
                target.clone(),
                UPDATE_MAP_DIRECTORY,
                "admin",
                "switch",
                Some(Duration::from_secs(2)),
                &request,
            )
            .await
            .unwrap();
        let CatalogResponse::Snapshot { snapshot: updated } = client
            .catalog_exact(server.peer_id(), route.clone(), Some(catalog.revision))
            .await
            .unwrap()
        else {
            panic!("default change must invalidate catalog")
        };
        assert!(updated.revision > catalog.revision);
        let updated_entry = updated
            .products
            .iter()
            .find(|p| p.manifest.reference() == directory.product().reference())
            .unwrap();
        assert_eq!(
            updated_entry.metadata.as_ref().unwrap().source_sequence,
            changed.result.sequence
        );
        let data: DomainMapDirectory =
            serde_json::from_value(updated_entry.metadata.as_ref().unwrap().value.clone()).unwrap();
        assert_eq!(data.default_map_id.as_deref(), Some("b"));
        let selected: Invocation<ResolvedMap> = client
            .invoke_exact(
                server.peer_id(),
                route,
                target,
                RESOLVE_MAP,
                "reader",
                "new-default",
                Some(Duration::from_secs(2)),
                &domain_only,
            )
            .await
            .unwrap();
        assert_eq!(selected.result.selected.product, b.product().reference());
        endpoint.close().await.unwrap();
        directory.close();
        a.close();
        b.close();
        consumer.shutdown().await.unwrap();
        server.shutdown().await.unwrap();
    })
    .await
    .expect("isolated directory test timed out");
}

#[path = "../../auki-components/tests/support/clock.rs"]
mod clock_fixture;
use clock_fixture::fixture_clock;
