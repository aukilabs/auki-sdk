use auki_identity::Wallet;
use auki_p2p::{
    AukiBrowserBootstrapRecord, AukiNodeBuilder, AukiServeRuntime, AukiServeRuntimeEvent,
    AukiServedInbound, LatestPublishedByteSource, PreviewOfferOptions, PublishedByteFrame,
    publish_preview_offer_with_latest_source,
};
use auki_protocol::v1::{domain::DOMAIN_NONCE_LEN, subscribe::SubscribeEndReason};
use jpeg_encoder::{ColorType, Encoder};
use serde_json::json;
use std::{env, error::Error, fmt, fs, path::PathBuf, process::ExitCode, time::Duration};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::time::Instant;
use tracing_subscriber::EnvFilter;

const DEFAULT_DOMAIN_LABEL: &str = "auki-p2p-preview-demo";
const DEFAULT_OFFER_ID: &str = "sentinel-preview";
const DEFAULT_PEER_LABEL: &str = "p2p-preview-sentinel";
const DEFAULT_STATUS_INTERVAL_MS: u64 = 2_000;
const DEFAULT_FRAME_INTERVAL_MS: u64 = 100;
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
    if config.trace_p2p {
        init_p2p_tracing();
    }

    let now = now_rfc3339()?;
    let wallet = Wallet::from_seed(vec![7; 32])?;
    let builder = AukiNodeBuilder::from_wallet(wallet, &now, Some(DEFAULT_PEER_LABEL))?
        .with_browser_reachable_development()
        .with_owner_domain([42; DOMAIN_NONCE_LEN], Some(DEFAULT_DOMAIN_LABEL), true)?;
    let domain_id = builder
        .primary_domain_id()
        .ok_or_else(|| CliError::new("preview sentinel requires one local domain"))?
        .to_owned();
    let mut node = builder.build(&now)?;
    let preview_source = LatestPublishedByteSource::new();
    let preview_producer = start_generated_preview_source(
        preview_source.clone(),
        config.frame_limit,
        config.frame_interval,
    );
    publish_preview_offer_with_latest_source(
        &mut node,
        preview_source.clone(),
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

    let bootstrap = node
        .wait_for_browser_bootstrap_record(Duration::from_secs(5), &now_rfc3339()?)
        .await;
    write_bootstrap_record(&bootstrap, config.bootstrap_json.as_ref())?;
    print_bootstrap_record(&bootstrap)?;

    let mut runtime = AukiServeRuntime::new(node);
    let mut last_failure = None;
    print_state(&mut runtime, last_failure.as_deref(), &domain_id, &config)?;
    if config.once {
        preview_source.close();
        preview_producer.abort();
        return Ok(());
    }

    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    let mut next_status_at = Instant::now() + config.status_interval;
    loop {
        let now = now_rfc3339()?;
        // Do not race status ticks against serve_next: dropping serve_next
        // after it accepts a stream resets that stream before it can reply.
        tokio::select! {
            _ = &mut shutdown => {
                println!("shutdown: ctrl-c received");
                preview_source.close();
                if let Err(error) = runtime
                    .shutdown_active_subscriptions(SubscribeEndReason::ProducerShutdown)
                    .await
                {
                    eprintln!("shutdown: subscription shutdown failed: {error}");
                }
                preview_producer.abort();
                break;
            }
            result = runtime.serve_next(&now) => {
                match result {
                    Ok(Some(event)) => {
                        if config.trace_p2p {
                            trace_runtime_event(&event)?;
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        if config.trace_p2p {
                            println!("trace: at={now} runtime_error={error}");
                        }
                        last_failure = Some(error.to_string());
                    }
                }
            }
        }

        let monotonic_now = Instant::now();
        if monotonic_now >= next_status_at {
            print_state(&mut runtime, last_failure.as_deref(), &domain_id, &config)?;
            while next_status_at <= monotonic_now {
                next_status_at += config.status_interval;
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
    status_interval: Duration,
    once: bool,
    trace_p2p: bool,
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
            status_interval: Duration::from_millis(DEFAULT_STATUS_INTERVAL_MS),
            once: false,
            trace_p2p: false,
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
                "--trace-p2p" => {
                    config.trace_p2p = true;
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
           --frames N                  Optional finite generated producer-frame limit. Default: continuous\n\
           --frame-interval-ms N       Generated stream frame interval. Default: 100\n\
           --status-interval-ms N      P2P state print interval. Default: 2000\n\
           --trace-p2p                 Print per-operation P2P trace lines for troubleshooting\n\
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

fn print_bootstrap_record(record: &AukiBrowserBootstrapRecord) -> Result<(), Box<dyn Error>> {
    let value = record.to_value();
    println!("browser_bootstrap_json:");
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn init_p2p_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("libp2p_stream=debug,libp2p_yamux=debug,auki_p2p=trace")
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn write_bootstrap_record(
    record: &AukiBrowserBootstrapRecord,
    path: Option<&PathBuf>,
) -> Result<(), Box<dyn Error>> {
    let Some(path) = path else {
        return Ok(());
    };
    let value = record.to_value();
    fs::write(path, serde_json::to_string_pretty(&value)?)?;
    println!("wrote bootstrap JSON: {}", path.display());
    Ok(())
}

fn print_state(
    runtime: &mut AukiServeRuntime,
    last_failure: Option<&str>,
    domain_id: &str,
    config: &DemoConfig,
) -> Result<(), Box<dyn Error>> {
    let now = now_rfc3339()?;
    let runtime_status = runtime.status().clone();
    let active_subscriptions = runtime.active_subscriptions();
    let (snapshot, offers, relationships, peer_id, dialable_addresses, browser_relay_addresses) = {
        let node = runtime.node_mut();
        (
            node.status_snapshot(&now)?,
            node.local_offers(domain_id),
            node.relationships(),
            node.peer_id(),
            node.observed_dialable_listen_addresses().len(),
            node.observed_browser_relay_server_addresses().len(),
        )
    };

    println!("--- p2p state @ {now} ---");
    println!(
        "local: peer_id={} domains={} offers={} dialable_addresses={} browser_relay_addresses={}",
        peer_id,
        snapshot.local_domains.len(),
        offers.len(),
        dialable_addresses,
        browser_relay_addresses,
    );
    println!(
        "preview: source={} offer_id={} frame_limit={} frame_interval_ms={} backpressure=LatestOnly",
        config.source.as_str(),
        config.offer_id,
        frame_limit_label(config.frame_limit),
        config.frame_interval.as_millis(),
    );
    println!(
        "serving: lifecycles={} duplicate_lifecycles={} offer_catalogs={} gets={} get_rejected={} subscriptions={} active_subscriptions={} subscription_rejected={} subscription_completed={} subscription_cancelled={} subscription_failed={} subscription_backpressure={} subscription_producer_closed={} frames_produced={} frames_sent={} frames_dropped={} slow_writes={}",
        runtime_status.lifecycles_served,
        runtime_status.duplicate_lifecycle_attempts,
        runtime_status.offer_catalogs_served,
        runtime_status.gets_served,
        runtime_status.gets_rejected,
        runtime_status.subscriptions_accepted,
        runtime_status.active_subscriptions,
        runtime_status.subscriptions_rejected,
        runtime_status.subscriptions_completed,
        runtime_status.subscriptions_cancelled,
        runtime_status.subscriptions_failed,
        runtime_status.subscriptions_closed_for_backpressure,
        runtime_status.subscriptions_closed_by_producer,
        runtime_status.frames_produced,
        runtime_status.frames_sent,
        runtime_status.frames_dropped,
        runtime_status.subscription_slow_writes,
    );
    println!(
        "raw_inbound: lifecycles={} offer_catalogs={} gets={} subscribes={} pending={} handoff_queue={} handoff_full={} handoff_closed={}",
        runtime_status.raw_inbound_lifecycles,
        runtime_status.raw_inbound_offer_catalogs,
        runtime_status.raw_inbound_gets,
        runtime_status.raw_inbound_subscribes,
        runtime_status.pending_inbound_streams,
        runtime_status.inbound_accept_queue_depth,
        runtime_status.inbound_accept_queue_full,
        runtime_status.inbound_accept_queue_closed,
    );
    if let Some(error) = last_failure {
        println!("last_failure: {error}");
    }
    if let Some(error) = &runtime_status.last_failure {
        println!("last_runtime_failure: {error}");
    }
    if !active_subscriptions.is_empty() {
        println!("active_subscriptions:");
        for subscription in &active_subscriptions {
            println!(
                "  id={} peer={} domain={} offer={} payload={} messages_sent={} policy={:?}",
                subscription.subscription_id,
                subscription.peer_id,
                subscription.domain_id,
                subscription.offer_id,
                subscription.payload_type,
                subscription.messages_sent,
                subscription.backpressure_policy,
            );
        }
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

fn trace_runtime_event(event: &AukiServeRuntimeEvent) -> Result<(), Box<dyn Error>> {
    let now = now_rfc3339()?;
    match event {
        AukiServeRuntimeEvent::Inbound(AukiServedInbound::Get(served)) => {
            println!(
                "trace: at={now} inbound=get peer={} domain={} offer={} success={} failure={}",
                served.peer_id,
                option_label(served.domain_id.as_deref()),
                option_label(served.offer_id.as_deref()),
                served.success,
                option_label(served.failure_code.as_deref()),
            );
        }
        AukiServeRuntimeEvent::Inbound(AukiServedInbound::Subscribe(served)) => {
            println!(
                "trace: at={now} inbound=subscribe peer={} domain={} offer={} accepted={} failure={}",
                served.peer_id,
                option_label(served.domain_id.as_deref()),
                option_label(served.offer_id.as_deref()),
                served.accepted,
                option_label(served.failure_code.as_deref()),
            );
        }
        AukiServeRuntimeEvent::Inbound(AukiServedInbound::OfferCatalog(served)) => {
            println!(
                "trace: at={now} inbound=offer_catalog peer={} domains={} kinds={} inline_registry={}",
                served.peer_id,
                served.domain_ids.len(),
                served.kinds.len(),
                served.include_inline_registry_entries,
            );
        }
        AukiServeRuntimeEvent::Inbound(AukiServedInbound::Lifecycle(_)) => {
            println!("trace: at={now} inbound=lifecycle");
        }
        AukiServeRuntimeEvent::PublishedSubscriptionStarted(status) => {
            println!(
                "trace: at={now} subscription_started id={} peer={} domain={} offer={} payload={} policy={:?}",
                status.subscription_id,
                status.peer_id,
                status.domain_id,
                status.offer_id,
                status.payload_type,
                status.backpressure_policy,
            );
        }
        AukiServeRuntimeEvent::PublishedSubscriptionMessageSent(_) => {}
        AukiServeRuntimeEvent::PublishedSubscriptionEnded(status) => {
            println!(
                "trace: at={now} subscription_ended id={} peer={} domain={} offer={} reason={:?} error={} retryable={} messages_sent={}",
                status.subscription_id,
                status.peer_id,
                status.domain_id,
                status.offer_id,
                status.reason,
                option_label(status.error_code.as_deref()),
                status
                    .retryable
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                status.messages_sent,
            );
        }
    }
    Ok(())
}

fn option_label(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}

fn frame_limit_label(frame_limit: Option<u64>) -> String {
    frame_limit
        .map(|limit| limit.to_string())
        .unwrap_or_else(|| "continuous".to_owned())
}

fn start_generated_preview_source(
    source: LatestPublishedByteSource,
    frame_limit: Option<u64>,
    frame_interval: Duration,
) -> tokio::task::JoinHandle<()> {
    source.publish(generated_preview_frame(0));
    tokio::spawn(async move {
        let mut sequence = 1_u64;
        let mut remaining = frame_limit.map(|limit| limit.saturating_sub(1));
        loop {
            if matches!(remaining, Some(0)) {
                source.close();
                return;
            }
            if !frame_interval.is_zero() {
                tokio::time::sleep(frame_interval).await;
            }
            if !source.publish(generated_preview_frame(sequence)) {
                return;
            }
            sequence = match sequence.checked_add(1) {
                Some(sequence) => sequence,
                None => {
                    source.close();
                    return;
                }
            };
            if let Some(remaining) = &mut remaining {
                *remaining = remaining.saturating_sub(1);
            }
        }
    })
}

fn generated_preview_frame(sequence: u64) -> PublishedByteFrame {
    let mut frame = PublishedByteFrame::new(generated_jpeg_frame(sequence)).with_sequence(sequence);
    if let Ok(generated_at) = now_rfc3339() {
        frame = frame.with_generated_at(generated_at);
    }
    frame
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
            config.status_interval,
            Duration::from_millis(DEFAULT_STATUS_INTERVAL_MS)
        );
        assert!(!config.trace_p2p);
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
            "--status-interval-ms".to_owned(),
            "250".to_owned(),
            "--trace-p2p".to_owned(),
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
        assert_eq!(config.status_interval, Duration::from_millis(250));
        assert!(config.trace_p2p);
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
    fn generated_preview_frame_carries_source_sequence() {
        let frame = generated_preview_frame(12);

        assert_eq!(frame.sequence, Some(12));
        assert!(frame.generated_at.is_some());
        assert!(frame.bytes.len() > 100);
        assert_eq!(&frame.bytes[..2], &[0xff, 0xd8]);
        assert_eq!(&frame.bytes[frame.bytes.len() - 2..], &[0xff, 0xd9]);
    }
}
