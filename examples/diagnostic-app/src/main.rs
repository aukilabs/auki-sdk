mod auth;
mod catalog;

use std::{
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use auki_domain::{Domain, DomainBuilder, DomainConfig, Multiaddr, PeerId};
use auki_session::Peer;
use auth::{create_demo_material, load_authority, require_empty_or_missing_directory};
use catalog::StaticCatalog;
use clap::{Args, Parser, Subcommand};
use tokio::time::{MissedTickBehavior, interval, sleep, sleep_until};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(
    name = "auki-diagnostic-app",
    about = "Run one host-configured authenticated Auki Domain over explicit routes"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run one Domain. Nothing is fetched from DDS, DMS, or a relay service.
    Run(Box<RunArgs>),
    /// Print the Peer ID encoded by a canonical stable identity file.
    PeerId(PeerIdArgs),
    /// Create explicitly insecure, short-lived files for the local demo only.
    DemoMaterial(DemoMaterialArgs),
}

#[derive(Debug, Args)]
struct RunArgs {
    /// Exact DDS Domain UUID.
    #[arg(long)]
    domain: Uuid,

    /// Canonical protobuf-encoded Ed25519 private identity.
    #[arg(long)]
    identity: PathBuf,

    /// Host-fetched DDS ES256 public key in PEM format.
    #[arg(long)]
    dds_public_key: PathBuf,

    /// Host-fetched compact, peer-bound P2P access JWT.
    #[arg(long)]
    credential: PathBuf,

    /// Monotonic generation assigned to the supplied DDS key set.
    #[arg(long, default_value_t = 0)]
    dds_key_generation: u64,

    /// Listener multiaddr. Repeat to bind more than one address.
    #[arg(long, default_value = "/ip4/127.0.0.1/tcp/0")]
    listen: Vec<Multiaddr>,

    /// Explicit dial hint as PEER_ID=MULTIADDR. Repeat for more candidates.
    #[arg(long = "route", value_parser = parse_route)]
    routes: Vec<PeerRoute>,

    /// Fetch this peer's authenticated v0.2 resource catalog. Repeatable.
    #[arg(long = "fetch-peer")]
    fetch_peers: Vec<PeerId>,

    /// Resource ID served by this process. Repeat to advertise multiple rows.
    #[arg(long = "resource", default_value = "diagnostic-camera")]
    resources: Vec<String>,

    /// Application storage used by the retained Peer and Session objects.
    #[arg(long)]
    storage: PathBuf,

    /// Bound for each remote catalog fetch, including route retries.
    #[arg(long, default_value_t = 15)]
    fetch_timeout_secs: u64,

    /// Keep serving briefly after all requested fetches finish.
    #[arg(long, default_value_t = 1_000)]
    post_fetch_grace_ms: u64,

    /// Without fetch targets, leave automatically after this many seconds.
    #[arg(long)]
    run_for_secs: Option<u64>,
}

#[derive(Debug, Args)]
struct PeerIdArgs {
    #[arg(long)]
    identity: PathBuf,
}

#[derive(Debug, Args)]
struct DemoMaterialArgs {
    #[arg(long)]
    output: PathBuf,

    #[arg(long)]
    domain: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PeerRoute {
    peer_id: PeerId,
    address: Multiaddr,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Run(args) => run(*args).await,
        Command::PeerId(args) => print_peer_id(args),
        Command::DemoMaterial(args) => print_demo_material(args),
    }
}

fn print_peer_id(args: PeerIdArgs) -> Result<()> {
    let bytes = std::fs::read(&args.identity)
        .with_context(|| format!("read identity {}", args.identity.display()))?;
    let identity = auki_domain::Identity::from_protobuf_encoding(&bytes)
        .with_context(|| format!("decode identity {}", args.identity.display()))?;
    println!("{}", identity.peer_id());
    Ok(())
}

fn print_demo_material(args: DemoMaterialArgs) -> Result<()> {
    require_empty_or_missing_directory(&args.output)?;
    let material = create_demo_material(&args.output, args.domain)?;
    println!(
        "MATERIAL directory={} domain_id={} wrong_domain_id={} peer_a={} peer_b={}",
        material.directory.display(),
        material.domain_id,
        material.wrong_domain_id,
        material.peer_a,
        material.peer_b
    );
    println!("WARNING demo credentials use a throwaway signing key and expire in 30 minutes");
    Ok(())
}

async fn run(args: RunArgs) -> Result<()> {
    if args.fetch_timeout_secs == 0 {
        bail!("--fetch-timeout-secs must be greater than zero");
    }
    if args.resources.iter().any(|resource| resource.is_empty()) {
        bail!("--resource values must not be empty");
    }

    let authority = load_authority(
        &args.identity,
        &args.dds_public_key,
        &args.credential,
        args.dds_key_generation,
    )?;
    std::fs::create_dir_all(&args.storage)
        .with_context(|| format!("create storage directory {}", args.storage.display()))?;

    let local_peer = authority.identity.peer_id();
    let peer = Peer::new(local_peer.to_string(), "auki-diagnostic-app")
        .with_storage_root(args.storage.clone());
    let session = peer.start_session().context("start diagnostic Session")?;
    let mut config = DomainConfig::new(args.domain, authority.identity)
        .with_listen_addresses(args.listen)
        .context("configure listeners")?;
    for (peer_id, addresses) in group_routes(args.routes) {
        config = config
            .with_peer_routes(peer_id, addresses)
            .with_context(|| format!("configure routes for {peer_id}"))?;
    }

    let provider = Arc::new(StaticCatalog::new(local_peer, &args.resources));
    let domain = match DomainBuilder::new(&peer, &session, config)
        .authority(authority.keys, authority.credential)
        .resource_catalog_provider(provider)
        .join()
        .await
    {
        Ok(domain) => domain,
        Err(error) => {
            eprintln!(
                "JOIN_FAILED domain_id={} peer_id={} error={error}",
                args.domain, local_peer
            );
            return Err(error).context("authenticated Domain join failed");
        }
    };

    let listeners = domain
        .listen_addresses()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "READY domain_id={} peer_id={} status={:?} listeners={listeners}",
        domain.domain_id(),
        domain.peer_id(),
        domain.status()
    );
    println!(
        "LOCAL_CATALOG count={} resource_ids={}",
        args.resources.len(),
        args.resources.join(",")
    );

    let operation = if args.fetch_peers.is_empty() {
        observe_until_stopped(&domain, args.run_for_secs).await
    } else {
        let result = fetch_catalogs(
            &domain,
            &args.fetch_peers,
            Duration::from_secs(args.fetch_timeout_secs),
        )
        .await;
        print_known_peers(&domain);
        if args.post_fetch_grace_ms != 0 {
            sleep(Duration::from_millis(args.post_fetch_grace_ms)).await;
        }
        result
    };

    println!("LEAVING peer_id={local_peer}");
    let leave = domain.leave().await.context("ordered Domain leave");
    if leave.is_ok() {
        println!("LEFT peer_id={local_peer}");
    }
    operation?;
    leave
}

async fn fetch_catalogs(domain: &Domain, targets: &[PeerId], timeout: Duration) -> Result<()> {
    for target in targets {
        let deadline = Instant::now() + timeout;
        loop {
            let last_error = match domain.fetch_resources_catalog(*target).await {
                Ok(catalog) => {
                    let ids = catalog
                        .resources
                        .iter()
                        .map(|resource| resource.resource_id.as_str())
                        .collect::<Vec<_>>()
                        .join(",");
                    println!(
                        "CATALOG peer_id={target} count={} resource_ids={ids}",
                        catalog.resources.len()
                    );
                    break;
                }
                Err(error) => error.to_string(),
            };
            if Instant::now() >= deadline {
                bail!("catalog fetch from {target} timed out: {last_error}");
            }
            sleep(Duration::from_millis(100)).await;
        }
    }
    Ok(())
}

async fn observe_until_stopped(domain: &Domain, run_for_secs: Option<u64>) -> Result<()> {
    let mut report = interval(Duration::from_secs(1));
    report.set_missed_tick_behavior(MissedTickBehavior::Skip);
    report.tick().await;
    match run_for_secs {
        Some(seconds) => {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
            loop {
                tokio::select! {
                    _ = sleep_until(deadline) => break,
                    _ = report.tick() => print_known_peers(domain),
                    signal = tokio::signal::ctrl_c() => {
                        signal.context("wait for Ctrl-C")?;
                        break;
                    }
                }
            }
        }
        None => loop {
            tokio::select! {
                _ = report.tick() => print_known_peers(domain),
                signal = tokio::signal::ctrl_c() => {
                    signal.context("wait for Ctrl-C")?;
                    break;
                }
            }
        },
    }
    Ok(())
}

fn print_known_peers(domain: &Domain) {
    let snapshot = domain.known_peers().snapshot();
    let ids = snapshot
        .peers()
        .iter()
        .map(|peer| peer.peer_id().to_string())
        .collect::<Vec<_>>()
        .join(",");
    println!("PEERS count={} peer_ids={ids}", snapshot.peers().len());
}

fn parse_route(value: &str) -> std::result::Result<PeerRoute, String> {
    let (peer_id, address) = value
        .split_once('=')
        .ok_or_else(|| "expected PEER_ID=MULTIADDR".to_owned())?;
    let peer_id = PeerId::from_str(peer_id).map_err(|error| format!("invalid Peer ID: {error}"))?;
    let address = Multiaddr::from_str(address)
        .map_err(|error| format!("invalid route multiaddr: {error}"))?;
    if address.is_empty() {
        return Err("route multiaddr must not be empty".into());
    }
    Ok(PeerRoute { peer_id, address })
}

fn group_routes(routes: Vec<PeerRoute>) -> Vec<(PeerId, Vec<Multiaddr>)> {
    let mut grouped: Vec<(PeerId, Vec<Multiaddr>)> = Vec::new();
    for route in routes {
        if let Some((_, addresses)) = grouped
            .iter_mut()
            .find(|(peer_id, _)| *peer_id == route.peer_id)
        {
            addresses.push(route.address);
        } else {
            grouped.push((route.peer_id, vec![route.address]));
        }
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_p2p::DdsTokenVerifier;

    #[test]
    fn route_parser_requires_an_expected_peer_and_nonempty_address() {
        let peer = auki_domain::Identity::from_ed25519_seed(&[7; 32]).peer_id();
        let parsed = parse_route(&format!("{peer}=/ip4/127.0.0.1/tcp/31001")).unwrap();
        assert_eq!(parsed.peer_id, peer);
        assert_eq!(parsed.address.to_string(), "/ip4/127.0.0.1/tcp/31001");

        assert!(parse_route("/ip4/127.0.0.1/tcp/31001").is_err());
        assert!(parse_route(&format!("{peer}=")).is_err());
    }

    #[test]
    fn demo_material_is_stable_signed_and_fail_closed_for_wrong_domain() {
        let temp = tempfile::tempdir().unwrap();
        let domain_id = Uuid::parse_str("11111111-2222-4333-8444-555555555555").unwrap();
        let material = create_demo_material(temp.path(), domain_id).unwrap();
        assert_ne!(material.domain_id, material.wrong_domain_id);

        let authority_a = load_authority(
            &temp.path().join("peer-a.identity"),
            &temp.path().join("dds-public.pem"),
            &temp.path().join("peer-a.jwt"),
            0,
        )
        .unwrap();
        assert_eq!(authority_a.identity.peer_id().to_string(), material.peer_a);

        let public_key = std::fs::read(temp.path().join("dds-public.pem")).unwrap();
        let verifier = DdsTokenVerifier::from_es256_pem(&public_key).unwrap();
        let compact = std::fs::read_to_string(temp.path().join("peer-a.jwt")).unwrap();
        let claims = verifier.verify(compact.trim()).unwrap();
        assert_eq!(claims.peer_id, material.peer_a);
        assert_eq!(claims.domain_ids, [domain_id.to_string()]);

        let wrong = std::fs::read_to_string(temp.path().join("peer-a-wrong-domain.jwt")).unwrap();
        let wrong_claims = verifier.verify(wrong.trim()).unwrap();
        assert_eq!(wrong_claims.peer_id, material.peer_a);
        assert!(!wrong_claims.domain_ids.contains(&domain_id.to_string()));
    }

    #[test]
    fn duplicate_route_candidates_are_grouped_per_expected_peer() {
        let peer = auki_domain::Identity::from_ed25519_seed(&[8; 32]).peer_id();
        let routes = [31001, 31002]
            .into_iter()
            .map(|port| parse_route(&format!("{peer}=/ip4/127.0.0.1/tcp/{port}")).unwrap())
            .collect();
        let grouped = group_routes(routes);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].0, peer);
        assert_eq!(grouped[0].1.len(), 2);
    }

    #[test]
    fn cli_rejects_manager_era_commands() {
        assert!(Cli::try_parse_from(["diagnostic", "cluster", "join"]).is_err());
        assert!(Cli::try_parse_from(["diagnostic", "discovery"]).is_err());
    }
}
