//! Explicit dev smoke check. See docs/how-to/domain-data.md before running.

use auki_sdk::{
    AukiDomainData, AukiDomains, AukiPeerBootstrap, AukiPeerConfig, AuthClient, AuthEnvironment,
    Credentials, DataListQuery, DataWrite, DomainListQuery,
};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let environment = AuthEnvironment::dev().with_client_id(std::env::var("AUKI_CLIENT_ID")?)?;
    let credentials = Credentials::user_password(
        std::env::var("AUKI_EMAIL")?,
        std::env::var("AUKI_PASSWORD")?,
    );
    // Trusted backends may instead use Credentials::app(access_key, secret).
    let credential = AuthClient::new(environment)?
        .authenticate(credentials)
        .await?;
    let result = run(credential.clone()).await;
    credential.close().await;
    result
}

async fn run(credential: auki_sdk::AukiCredential) -> Result<(), Box<dyn std::error::Error>> {
    let domains = AukiDomains::new(credential.clone());
    let page = domains.list(&DomainListQuery::default()).await?;
    println!(
        "Listed {} of {} Domains without requesting per-Domain tokens",
        page.domains.len(),
        page.total
    );
    let domain_id: Uuid = std::env::var("AUKI_DOMAIN_ID")?.parse()?;
    let clients = AukiDomainData::new(credential.clone())?;
    let data = clients.in_domain(domain_id);
    let peer = if std::env::args().any(|arg| arg == "--with-peer") {
        Some(
            AukiPeerBootstrap::from_session(credential, AukiPeerConfig::dev().direct_only())
                .start_ephemeral_peer(domain_id.into())
                .await?,
        )
    } else {
        None
    };
    let name = format!("sdk-374-{}", Uuid::new_v4());
    println!("Temporary record name: {name}");
    let filter = DataListQuery {
        name: Some(name.clone()),
        ..Default::default()
    };
    let result: Result<(), Box<dyn std::error::Error>> = async {
        let saved = data
            .write(
                DataWrite::Named {
                    name: &name,
                    data_type: "sdk-test.report.v1",
                },
                b"SDK Domain data smoke check",
            )
            .await.map_err(|error| format!("create temporary record: {error}"))?;
        check(
            data.get(saved.id).await?.domain_id == domain_id,
            "metadata Domain mismatch",
        )?;
        check(
            data.read(saved.id).await? == b"SDK Domain data smoke check",
            "initial read mismatch",
        )?;
        data.write(DataWrite::ById(saved.id), b"updated by ID")
            .await.map_err(|error| format!("replace by ID: {error}"))?;
        check(
            data.read(saved.id).await? == b"updated by ID",
            "ID replacement mismatch",
        )?;
        let duplicate = data
            .write(
                DataWrite::Named {
                    name: &name,
                    data_type: "sdk-test.report.v1",
                },
                b"updated by name",
            )
            .await;
        check(
            duplicate.as_ref().err().and_then(auki_sdk::DataError::status) == Some(409),
            "expected duplicate-name conflict",
        )?;
        check(
            data.read(saved.id).await? == b"updated by ID",
            "conflicting upload changed existing bytes",
        )?;
        check(
            data.list(&filter).await?.len() == 1,
            "metadata filter mismatch",
        )?;
        if std::env::args().any(|arg| arg == "--stream") {
            streaming_round_trip(&data, &domains, domain_id, saved.id).await?;
            data.write(DataWrite::ById(saved.id), b"updated by ID").await?;
        }
        data.close().await;
        if let Some(peer) = &peer {
            check(peer.status().is_ready(), "closing data stopped the peer")?;
        }
        check(
            clients.in_domain(domain_id).read(saved.id).await? == b"updated by ID",
            "shared credential stopped",
        )?;
        println!(
            "Read, filters, named upload, duplicate conflict, ID replacement and shared ownership passed"
        );
        Ok(())
    }
    .await;
    // Reconcile by the unique name even if the write response was lost.
    // Never delete pre-existing application records or retry an ambiguous write.
    let cleanup: Result<(), auki_sdk::DataError> = async {
        let cleanup_client = clients.in_domain(domain_id);
        for record in cleanup_client.list(&filter).await? {
            cleanup_client.delete(record.id).await?;
        }
        if !cleanup_client.list(&filter).await?.is_empty() {
            return Err(auki_sdk::DataError::InvalidResponse(
                "temporary records remain",
            ));
        }
        cleanup_client.close().await;
        Ok(())
    }
    .await;
    data.close().await;
    let peer_cleanup = match peer {
        Some(peer) => peer.shutdown().await,
        None => Ok(()),
    };
    if let Err(error) = &cleanup {
        eprintln!("Cleanup failed for {name}: {error}");
    }
    cleanup?;
    peer_cleanup?;
    result?;
    println!("Temporary data deleted; clients and optional peer stopped");
    Ok(())
}

fn check(condition: bool, message: &'static str) -> Result<(), Box<dyn std::error::Error>> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

async fn streaming_round_trip(
    data: &auki_sdk::DomainDataClient,
    domains: &AukiDomains,
    domain: Uuid,
    id: Uuid,
) -> Result<(), Box<dyn std::error::Error>> {
    use auki_sdk::{PortalId, TransferOptions};
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    let cancel = tokio_util::sync::CancellationToken::new();
    let portals = domains.portals(domain, &cancel).await?;
    let poses = data.poses(&cancel).await?;
    if let Some(portal) = portals.first() {
        domains
            .portal(domain, &PortalId::parse(&portal.id.to_string())?, &cancel)
            .await?;
        domains
            .for_portal(&PortalId::parse(&portal.short_id)?, "own", &cancel)
            .await?;
    }
    if let Some(pose) = poses.first() {
        data.pose(&PortalId::parse(&pose.id.to_string())?, &cancel)
            .await?;
    }
    let size = 16 * 1024 * 1024 + 17u64;
    let mut remaining = size;
    data.write_stream(
        DataWrite::ById(id),
        size,
        TransferOptions::default(),
        &cancel,
        move |maximum| {
            let length = remaining.min(maximum as u64) as usize;
            remaining -= length as u64;
            std::future::ready(Ok(vec![b'a'; length]))
        },
    )
    .await?;
    let received = Arc::new(AtomicU64::new(0));
    let sink = received.clone();
    let downloaded = data
        .read_to(
            id,
            TransferOptions {
                max_bytes: size,
                max_chunk_bytes: 65536,
            },
            &cancel,
            move |bytes| {
                sink.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                std::future::ready(if bytes.iter().all(|byte| *byte == b'a') {
                    Ok(())
                } else {
                    Err(auki_sdk::DataError::Callback)
                })
            },
        )
        .await?;
    check(
        downloaded == size && received.load(Ordering::Relaxed) == size,
        "stream size mismatch",
    )?;
    println!(
        "Streamed {size} bytes; verified portal/pose reads ({} portals, {} poses)",
        portals.len(),
        poses.len()
    );
    Ok(())
}
