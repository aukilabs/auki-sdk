use auki_identity::Wallet;
use auki_p2p::{
    AukiGetProviderError, AukiNode, AukiNodeError, AukiNodeEvent, AukiP2pNodeConfig,
    AukiServedSubscription, LifecycleInput, LifecycleStreamDirection, LifecycleStreamGuardError,
    LocalDomainRegistration, LocalPeerIdentity, PreviewOfferOptions, PublishedByteSource,
    ServedGet, ServedSubscribe, preview_spatial_message, publish_preview_offer_with_snapshot,
};
use auki_protocol::v1::{
    domain::{DOMAIN_NONCE_LEN, DomainDeclaration},
    subscribe::SubscribeEndReason,
};
use futures::stream;
use jpeg_encoder::{ColorType, Encoder};
use serde_json::json;
use std::{
    env,
    error::Error,
    fmt, fs,
    path::PathBuf,
    process::ExitCode,
    time::{Duration, Instant},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::time::{MissedTickBehavior, timeout};

const DEFAULT_DOMAIN_LABEL: &str = "auki-p2p-preview-demo";
const DEFAULT_OFFER_ID: &str = "sentinel-preview";
const DEFAULT_PEER_LABEL: &str = "p2p-preview-sentinel";
const DEFAULT_STATUS_INTERVAL_MS: u64 = 2_000;
const DEFAULT_FRAME_INTERVAL_MS: u64 = 100;
const DEFAULT_LIFECYCLE_POLL_MS: u64 = 10;
const FRAME_WIDTH: u16 = 160;
const FRAME_HEIGHT: u16 = 90;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[tokio::main]
async fn run() -> Result<(), Box<dyn Error>> {
    let config = DemoConfig::parse(env::args().skip(1))?;
    if config.help {
        println!("{}", DemoConfig::usage());
        return Ok(());
    }

    let now = now_rfc3339()?;
    let wallet = Wallet::from_seed(vec![7; 32])?;
    let identity = LocalPeerIdentity::from_wallet(wallet.clone(), &now, Some(DEFAULT_PEER_LABEL))?;
    let declaration =
        DomainDeclaration::create(&wallet, &[42; DOMAIN_NONCE_LEN], Some(DEFAULT_DOMAIN_LABEL))?;
    let registration = LocalDomainRegistration::owner(declaration, true)?;
    let domain_id = registration.domain_id().to_owned();

    let mut node = AukiNode::new(
        identity,
        AukiP2pNodeConfig::loopback_browser_reachable_development(),
    )?;
    node.upsert_local_domain(registration, &now)?;
    let mut snapshot_sequence = 0_u64;
    publish_preview_offer_with_snapshot(
        &mut node,
        generated_jpeg_source(config.frame_limit, config.frame_interval),
        move |_request, _now| {
            let frame = generated_jpeg_frame(snapshot_sequence);
            snapshot_sequence = snapshot_sequence.saturating_add(1);
            Ok::<Vec<u8>, AukiGetProviderError>(frame)
        },
        PreviewOfferOptions::new(domain_id.clone(), config.offer_id.clone())
            .with_display_name("Sentinel preview")
            .with_metadata(json!({
                "example": "p2p-preview-sentinel",
                "source": config.source.as_str(),
                "frame_width": FRAME_WIDTH,
                "frame_height": FRAME_HEIGHT,
                "frame_limit": config.frame_limit,
                "frame_interval_ms": config.frame_interval.as_millis(),
            })),
    )?;

    println!("Auki P2P preview sentinel");
    println!("peer_id: {}", node.peer_id());
    println!("domain_id: {domain_id}");
    println!("offer_id: {}", config.offer_id);
    println!("source: {}", config.source.as_str());
    println!("waiting for browser-reachable listen addresses...");

    wait_for_bootstrap_addresses(&mut node, Duration::from_secs(5)).await?;
    write_bootstrap_record(&node, config.bootstrap_json.as_ref())?;
    print_bootstrap_record(&node)?;

    let mut stats = DemoStats::default();
    let mut active_subscriptions = Vec::new();
    print_state(
        &mut node,
        &stats,
        &active_subscriptions,
        &domain_id,
        &config,
    )?;
    if config.once {
        return Ok(());
    }

    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    let mut frame_tick = tokio::time::interval(config.frame_interval);
    frame_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut next_state_print = Instant::now() + config.status_interval;
    loop {
        let now = now_rfc3339()?;
        tokio::select! {
            _ = &mut shutdown => {
                println!("shutdown: ctrl-c received");
                end_active_subscriptions(&mut node, &mut active_subscriptions).await;
                break;
            }
            _ = frame_tick.tick(), if !active_subscriptions.is_empty() => {
                let now = now_rfc3339()?;
                if let Err(error) = send_active_preview_frames(
                    &mut node,
                    &mut active_subscriptions,
                    &mut stats,
                    &now,
                ).await {
                    stats.record_error(error.to_string());
                }
                if let Err(error) = pump_connection_lifecycle(
                    &mut node,
                    &mut stats,
                    config.lifecycle_poll,
                ).await {
                    stats.record_error(error.to_string());
                }
                if Instant::now() >= next_state_print {
                    print_state(&mut node, &stats, &active_subscriptions, &domain_id, &config)?;
                    next_state_print = Instant::now() + config.status_interval;
                }
            }
            result = serve_inbound_once(
                &mut node,
                &now,
                &mut stats,
                &mut active_subscriptions,
                config.frame_limit,
            ) => {
                if let Err(error) = result {
                    stats.record_error(error.to_string());
                }
                if Instant::now() >= next_state_print {
                    print_state(&mut node, &stats, &active_subscriptions, &domain_id, &config)?;
                    next_state_print = Instant::now() + config.status_interval;
                }
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceMode {
    Generated,
}

impl SourceMode {
    fn parse(value: &str) -> Result<Self, CliError> {
        match value {
            "generated" => Ok(Self::Generated),
            "camera" => Err(CliError::new(
                "camera source is planned; use --source generated for this first demo slice",
            )),
            other => Err(CliError::new(format!(
                "unsupported source '{other}'; expected generated"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Generated => "generated",
        }
    }
}

#[derive(Debug, Clone)]
struct DemoConfig {
    source: SourceMode,
    offer_id: String,
    bootstrap_json: Option<PathBuf>,
    frame_limit: Option<u64>,
    frame_interval: Duration,
    lifecycle_poll: Duration,
    status_interval: Duration,
    once: bool,
    help: bool,
}

impl Default for DemoConfig {
    fn default() -> Self {
        Self {
            source: SourceMode::Generated,
            offer_id: DEFAULT_OFFER_ID.to_owned(),
            bootstrap_json: None,
            frame_limit: None,
            frame_interval: Duration::from_millis(DEFAULT_FRAME_INTERVAL_MS),
            lifecycle_poll: Duration::from_millis(DEFAULT_LIFECYCLE_POLL_MS),
            status_interval: Duration::from_millis(DEFAULT_STATUS_INTERVAL_MS),
            once: false,
            help: false,
        }
    }
}

impl DemoConfig {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, CliError> {
        let mut config = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => {
                    config.help = true;
                }
                "--once" => {
                    config.once = true;
                }
                "--source" => {
                    let value = next_value("--source", &mut args)?;
                    config.source = SourceMode::parse(&value)?;
                }
                "--offer-id" => {
                    config.offer_id = next_value("--offer-id", &mut args)?;
                }
                "--bootstrap-json" => {
                    config.bootstrap_json =
                        Some(PathBuf::from(next_value("--bootstrap-json", &mut args)?));
                }
                "--frames" => {
                    config.frame_limit = Some(parse_positive_u64(
                        "--frames",
                        &next_value("--frames", &mut args)?,
                    )?);
                }
                "--frame-interval-ms" => {
                    let millis = parse_positive_u64(
                        "--frame-interval-ms",
                        &next_value("--frame-interval-ms", &mut args)?,
                    )?;
                    config.frame_interval = Duration::from_millis(millis);
                }
                "--lifecycle-poll-ms" => {
                    let millis = parse_positive_u64(
                        "--lifecycle-poll-ms",
                        &next_value("--lifecycle-poll-ms", &mut args)?,
                    )?;
                    config.lifecycle_poll = Duration::from_millis(millis);
                }
                "--status-interval-ms" => {
                    let millis = parse_positive_u64(
                        "--status-interval-ms",
                        &next_value("--status-interval-ms", &mut args)?,
                    )?;
                    config.status_interval = Duration::from_millis(millis);
                }
                other => {
                    return Err(CliError::new(format!(
                        "unknown argument '{other}'\n\n{}",
                        Self::usage()
                    )));
                }
            }
        }
        Ok(config)
    }

    fn usage() -> &'static str {
        "Usage: cargo run -p auki-p2p-preview-sentinel -- [options]\n\
         \n\
         Options:\n\
           --source generated          Preview source. Camera capture is planned but not wired yet.\n\
           --offer-id ID               Offer id to publish. Default: sentinel-preview\n\
           --bootstrap-json PATH       Write browser bootstrap JSON to PATH after listeners bind.\n\
           --frames N                  Optional finite generated-frame limit per subscription. Default: continuous\n\
           --frame-interval-ms N       Generated stream frame interval. Default: 100\n\
           --lifecycle-poll-ms N       Swarm poll budget after each streamed frame. Default: 10\n\
           --status-interval-ms N      P2P state print interval. Default: 2000\n\
           --once                      Print bootstrap/state once and exit.\n\
           -h, --help                  Show this help text"
    }
}

fn next_value(
    flag: &'static str,
    args: &mut impl Iterator<Item = String>,
) -> Result<String, CliError> {
    args.next()
        .filter(|value| !value.starts_with('-'))
        .ok_or_else(|| CliError::new(format!("{flag} requires a value")))
}

fn parse_positive_u64(flag: &'static str, value: &str) -> Result<u64, CliError> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| CliError::new(format!("{flag} expects a positive integer")))?;
    if parsed == 0 {
        return Err(CliError::new(format!("{flag} expects a positive integer")));
    }
    Ok(parsed)
}

#[derive(Debug, Clone)]
struct CliError {
    message: String,
}

impl CliError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for CliError {}

#[derive(Debug, Default)]
struct DemoStats {
    lifecycles_served: u64,
    duplicate_lifecycle_attempts: u64,
    offer_catalogs_served: u64,
    gets_served: u64,
    gets_rejected: u64,
    subscriptions_served: u64,
    subscriptions_rejected: u64,
    subscriptions_closed: u64,
    frames_sent: u64,
    lifecycle_polls: u64,
    lifecycle_events: u64,
    last_failure: Option<String>,
}

impl DemoStats {
    fn record_get(&mut self, served: ServedGet) {
        if served.success {
            self.gets_served = self.gets_served.saturating_add(1);
        } else {
            self.gets_rejected = self.gets_rejected.saturating_add(1);
            self.last_failure = served.failure_code;
        }
    }

    fn record_subscribe_start(&mut self, served: &ServedSubscribe) {
        if served.accepted {
            self.subscriptions_served = self.subscriptions_served.saturating_add(1);
        } else {
            self.subscriptions_rejected = self.subscriptions_rejected.saturating_add(1);
            self.last_failure = served.failure_code.clone();
        }
    }

    fn record_error(&mut self, error: String) {
        self.last_failure = Some(error);
    }

    fn record_nonfatal_lifecycle_error(&mut self, error: &AukiNodeError) -> bool {
        if !is_duplicate_inbound_lifecycle(error) {
            return false;
        }
        self.duplicate_lifecycle_attempts = self.duplicate_lifecycle_attempts.saturating_add(1);
        true
    }
}

struct ActivePreviewSubscription {
    subscription: AukiServedSubscription,
    next_sequence: u64,
    remaining_frames: Option<u64>,
}

async fn serve_inbound_once(
    node: &mut AukiNode,
    now: &str,
    stats: &mut DemoStats,
    active_subscriptions: &mut Vec<ActivePreviewSubscription>,
    frame_limit: Option<u64>,
) -> Result<(), Box<dyn Error>> {
    match timeout(
        Duration::from_millis(100),
        node.serve_next_lifecycle(LifecycleInput::new(), now),
    )
    .await
    {
        Ok(Ok(Some(_))) => {
            stats.lifecycles_served = stats.lifecycles_served.saturating_add(1);
            return Ok(());
        }
        Ok(Ok(None)) | Err(_) => {}
        Ok(Err(error)) if stats.record_nonfatal_lifecycle_error(&error) => return Ok(()),
        Ok(Err(error)) => return Err(Box::new(error)),
    }

    match timeout(
        Duration::from_millis(100),
        node.serve_next_offer_catalog(Some(now)),
    )
    .await
    {
        Ok(Ok(Some(_))) => {
            stats.offer_catalogs_served = stats.offer_catalogs_served.saturating_add(1);
            return Ok(());
        }
        Ok(Ok(None)) | Err(_) => {}
        Ok(Err(error)) => return Err(Box::new(error)),
    }

    match timeout(Duration::from_millis(100), node.serve_next_get(now)).await {
        Ok(Ok(Some(served))) => {
            stats.record_get(served);
            return Ok(());
        }
        Ok(Ok(None)) | Err(_) => {}
        Ok(Err(error)) => return Err(Box::new(error)),
    }

    match timeout(Duration::from_millis(100), node.serve_next_subscribe(now)).await {
        Ok(Ok(Some(served))) => {
            stats.record_subscribe_start(&served);
            if served.accepted {
                let subscription = served
                    .into_subscription()
                    .ok_or_else(|| CliError::new("accepted Subscribe did not include a stream"))?;
                active_subscriptions.push(ActivePreviewSubscription {
                    subscription,
                    next_sequence: 0,
                    remaining_frames: frame_limit,
                });
            }
            Ok(())
        }
        Ok(Ok(None)) | Err(_) => Ok(()),
        Ok(Err(error)) => Err(Box::new(error)),
    }
}

async fn send_active_preview_frames(
    node: &mut AukiNode,
    active_subscriptions: &mut Vec<ActivePreviewSubscription>,
    stats: &mut DemoStats,
    now: &str,
) -> Result<(), Box<dyn Error>> {
    let mut index = 0;
    while index < active_subscriptions.len() {
        if active_subscriptions[index].remaining_frames == Some(0) {
            let active = active_subscriptions.remove(index);
            node.end_served_subscription(
                active.subscription,
                SubscribeEndReason::Complete,
                None,
                None,
            )
            .await?;
            stats.subscriptions_closed = stats.subscriptions_closed.saturating_add(1);
            continue;
        }

        let sequence = active_subscriptions[index].next_sequence;
        let frame = generated_jpeg_frame(sequence);
        let message = {
            let subscription = &active_subscriptions[index].subscription;
            preview_spatial_message(
                subscription.domain_id().to_owned(),
                subscription.offer_id().to_owned(),
                sequence,
                frame.as_slice(),
                Some(now),
            )?
        };

        let send_result = {
            let subscription = &mut active_subscriptions[index].subscription;
            node.send_served_subscription_message(subscription, &message)
                .await
        };

        match send_result {
            Ok(()) => {
                active_subscriptions[index].next_sequence =
                    active_subscriptions[index].next_sequence.saturating_add(1);
                if let Some(remaining) = &mut active_subscriptions[index].remaining_frames {
                    *remaining = remaining.saturating_sub(1);
                }
                stats.frames_sent = stats.frames_sent.saturating_add(1);
                index += 1;
            }
            Err(error) => {
                stats.subscriptions_closed = stats.subscriptions_closed.saturating_add(1);
                stats.record_error(error.to_string());
                active_subscriptions.remove(index);
            }
        }
    }

    Ok(())
}

async fn pump_connection_lifecycle(
    node: &mut AukiNode,
    stats: &mut DemoStats,
    max_wait: Duration,
) -> Result<(), Box<dyn Error>> {
    stats.lifecycle_polls = stats.lifecycle_polls.saturating_add(1);
    let started_at = Instant::now();

    while started_at.elapsed() < max_wait {
        let remaining = max_wait
            .checked_sub(started_at.elapsed())
            .unwrap_or_default();
        if remaining.is_zero() {
            break;
        }

        let now = now_rfc3339()?;
        match timeout(remaining, node.next_event(&now)).await {
            Ok(Some(event)) => {
                stats.lifecycle_events = stats.lifecycle_events.saturating_add(1);
                print_event(event);
            }
            Ok(None) | Err(_) => break,
        }
    }

    Ok(())
}

async fn end_active_subscriptions(
    node: &mut AukiNode,
    active_subscriptions: &mut Vec<ActivePreviewSubscription>,
) {
    while let Some(active) = active_subscriptions.pop() {
        let _ = node
            .end_served_subscription(
                active.subscription,
                SubscribeEndReason::OfferWithdrawn,
                None,
                None,
            )
            .await;
    }
}

fn is_duplicate_inbound_lifecycle(error: &AukiNodeError) -> bool {
    matches!(
        error,
        AukiNodeError::LifecycleGuard(LifecycleStreamGuardError {
            direction: LifecycleStreamDirection::Inbound,
            ..
        })
    )
}

async fn wait_for_bootstrap_addresses(
    node: &mut AukiNode,
    max_wait: Duration,
) -> Result<(), Box<dyn Error>> {
    let started_at = Instant::now();
    let expected_addresses = node.configured_listen_addresses().len().max(1);
    while node.browser_bootstrap_record().bootstrap_addresses.len() < expected_addresses
        && started_at.elapsed() < max_wait
    {
        let now = now_rfc3339()?;
        match timeout(Duration::from_millis(500), node.next_event(&now)).await {
            Ok(Some(event)) => print_event(event),
            Ok(None) | Err(_) => {}
        }
    }
    Ok(())
}

fn print_event(event: AukiNodeEvent) {
    match event {
        AukiNodeEvent::Listening { address } => {
            println!("event: listening {address}");
        }
        AukiNodeEvent::PeerConnected { peer_id } => {
            println!("event: peer connected {peer_id}");
        }
        AukiNodeEvent::PeerConnectionClosed {
            peer_id,
            active_connections,
        } => {
            println!("event: peer connection closed {peer_id} active={active_connections}");
        }
        AukiNodeEvent::PeerDuplicateConnectionClosed { peer_id } => {
            println!("event: duplicate connection closed {peer_id}");
        }
        AukiNodeEvent::PeerDialFailed { peer_id, error } => {
            println!("event: dial failed peer={peer_id:?} error={error}");
        }
        AukiNodeEvent::IncomingConnectionFailed { error } => {
            println!("event: incoming connection failed error={error}");
        }
    }
}

fn print_bootstrap_record(node: &AukiNode) -> Result<(), Box<dyn Error>> {
    let value = node.browser_bootstrap_record().to_value();
    println!("browser_bootstrap_json:");
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn write_bootstrap_record(node: &AukiNode, path: Option<&PathBuf>) -> Result<(), Box<dyn Error>> {
    let Some(path) = path else {
        return Ok(());
    };
    let value = node.browser_bootstrap_record().to_value();
    fs::write(path, serde_json::to_string_pretty(&value)?)?;
    println!("wrote bootstrap JSON: {}", path.display());
    Ok(())
}

fn print_state(
    node: &mut AukiNode,
    stats: &DemoStats,
    active_subscriptions: &[ActivePreviewSubscription],
    domain_id: &str,
    config: &DemoConfig,
) -> Result<(), Box<dyn Error>> {
    let now = now_rfc3339()?;
    let snapshot = node.status_snapshot(&now)?;
    let offers = node.local_offers(domain_id);
    let relationships = node.relationships();

    println!("--- p2p state @ {now} ---");
    println!(
        "local: peer_id={} domains={} offers={} dialable_addresses={} browser_relay_addresses={}",
        node.peer_id(),
        snapshot.local_domains.len(),
        offers.len(),
        node.observed_dialable_listen_addresses().len(),
        node.observed_browser_relay_server_addresses().len(),
    );
    println!(
        "preview: source={} offer_id={} frame_limit={} frame_interval_ms={} lifecycle_poll_ms={}",
        config.source.as_str(),
        config.offer_id,
        frame_limit_label(config.frame_limit),
        config.frame_interval.as_millis(),
        config.lifecycle_poll.as_millis(),
    );
    println!(
        "serving: lifecycles={} duplicate_lifecycles={} offer_catalogs={} gets={} get_rejected={} subscriptions={} active_subscriptions={} subscription_rejected={} subscription_closed={} frames_sent={} lifecycle_polls={} lifecycle_events={}",
        stats.lifecycles_served,
        stats.duplicate_lifecycle_attempts,
        stats.offer_catalogs_served,
        stats.gets_served,
        stats.gets_rejected,
        stats.subscriptions_served,
        active_subscriptions.len(),
        stats.subscriptions_rejected,
        stats.subscriptions_closed,
        stats.frames_sent,
        stats.lifecycle_polls,
        stats.lifecycle_events,
    );
    if let Some(error) = &stats.last_failure {
        println!("last_failure: {error}");
    }

    if relationships.is_empty() {
        println!("peers: none");
    } else {
        println!("peers:");
        for relationship in relationships {
            println!(
                "  {} state={} connected={} authorized={} transport_paths={} data_paths={} loaded_offers={}",
                relationship.peer_id,
                relationship.state,
                relationship.connected,
                relationship.authorized,
                relationship.transport_paths.len(),
                relationship.paths.len(),
                relationship.loaded_offers.len(),
            );
            for (index, path) in relationship.transport_paths.iter().enumerate() {
                println!(
                    "    transport[{index}] direction={} kind={} relay={}",
                    path.direction.as_str(),
                    path.transport.as_str(),
                    path.relay_involved
                );
            }
        }
    }

    Ok(())
}

fn frame_limit_label(frame_limit: Option<u64>) -> String {
    frame_limit
        .map(|limit| limit.to_string())
        .unwrap_or_else(|| "continuous".to_owned())
}

fn generated_jpeg_source(
    frame_limit: Option<u64>,
    frame_interval: Duration,
) -> impl FnMut() -> PublishedByteSource + Send + 'static {
    move || {
        Box::pin(stream::unfold(
            (0_u64, frame_limit),
            move |(sequence, remaining)| async move {
                match remaining {
                    Some(0) => None,
                    Some(limit) => {
                        sleep_before_next_frame(sequence, frame_interval).await;
                        Some((
                            generated_jpeg_frame(sequence),
                            (sequence.saturating_add(1), Some(limit.saturating_sub(1))),
                        ))
                    }
                    None => {
                        sleep_before_next_frame(sequence, frame_interval).await;
                        Some((
                            generated_jpeg_frame(sequence),
                            (sequence.saturating_add(1), None),
                        ))
                    }
                }
            },
        ))
    }
}

async fn sleep_before_next_frame(sequence: u64, frame_interval: Duration) {
    if sequence > 0 && !frame_interval.is_zero() {
        tokio::time::sleep(frame_interval).await;
    }
}

fn generated_jpeg_frame(sequence: u64) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(FRAME_WIDTH as usize * FRAME_HEIGHT as usize * 3);
    for y in 0..FRAME_HEIGHT {
        for x in 0..FRAME_WIDTH {
            rgb.push(((u64::from(x) * 2 + sequence * 5) % 256) as u8);
            rgb.push(((u64::from(y) * 3 + sequence * 7) % 256) as u8);
            rgb.push(((u64::from(x) + u64::from(y) + sequence * 11) % 256) as u8);
        }
    }

    let mut jpeg = Vec::new();
    Encoder::new(&mut jpeg, 80)
        .encode(&rgb, FRAME_WIDTH, FRAME_HEIGHT, ColorType::Rgb)
        .expect("generated RGB frame must encode as JPEG");
    jpeg
}

fn now_rfc3339() -> Result<String, time::error::Format> {
    OffsetDateTime::now_utc().format(&Rfc3339)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_defaults_use_generated_preview() {
        let config = DemoConfig::parse(Vec::<String>::new()).expect("defaults parse");

        assert_eq!(config.source, SourceMode::Generated);
        assert_eq!(config.offer_id, DEFAULT_OFFER_ID);
        assert_eq!(config.frame_limit, None);
        assert_eq!(
            config.frame_interval,
            Duration::from_millis(DEFAULT_FRAME_INTERVAL_MS)
        );
        assert_eq!(
            config.lifecycle_poll,
            Duration::from_millis(DEFAULT_LIFECYCLE_POLL_MS)
        );
        assert_eq!(
            config.status_interval,
            Duration::from_millis(DEFAULT_STATUS_INTERVAL_MS)
        );
    }

    #[test]
    fn parse_bootstrap_and_stream_options() {
        let config = DemoConfig::parse([
            "--bootstrap-json".to_owned(),
            "/tmp/bootstrap.json".to_owned(),
            "--offer-id".to_owned(),
            "camera-0".to_owned(),
            "--frames".to_owned(),
            "3".to_owned(),
            "--frame-interval-ms".to_owned(),
            "40".to_owned(),
            "--lifecycle-poll-ms".to_owned(),
            "15".to_owned(),
            "--status-interval-ms".to_owned(),
            "250".to_owned(),
            "--once".to_owned(),
        ])
        .expect("explicit options parse");

        assert_eq!(
            config.bootstrap_json,
            Some(PathBuf::from("/tmp/bootstrap.json"))
        );
        assert_eq!(config.offer_id, "camera-0");
        assert_eq!(config.frame_limit, Some(3));
        assert_eq!(config.frame_interval, Duration::from_millis(40));
        assert_eq!(config.lifecycle_poll, Duration::from_millis(15));
        assert_eq!(config.status_interval, Duration::from_millis(250));
        assert!(config.once);
    }

    #[test]
    fn parse_rejects_camera_until_adapter_exists() {
        let error =
            DemoConfig::parse(["--source".to_owned(), "camera".to_owned()]).expect_err("rejects");

        assert!(
            error
                .to_string()
                .contains("camera source is planned; use --source generated")
        );
    }

    #[test]
    fn generated_frame_is_jpeg() {
        let frame = generated_jpeg_frame(12);

        assert!(frame.len() > 100);
        assert_eq!(&frame[..2], &[0xff, 0xd8]);
        assert_eq!(&frame[frame.len() - 2..], &[0xff, 0xd9]);
    }

    #[test]
    fn duplicate_inbound_lifecycle_is_counted_without_failure() {
        let wallet = Wallet::from_seed(vec![9; 32]).expect("wallet");
        let identity = LocalPeerIdentity::from_wallet(wallet, "2026-05-27T00:00:00Z", Some("test"))
            .expect("identity");
        let error = AukiNodeError::LifecycleGuard(LifecycleStreamGuardError {
            peer_id: identity.peer_id(),
            direction: LifecycleStreamDirection::Inbound,
        });
        let mut stats = DemoStats::default();

        assert!(stats.record_nonfatal_lifecycle_error(&error));
        assert_eq!(stats.duplicate_lifecycle_attempts, 1);
        assert_eq!(stats.last_failure, None);
    }
}
