use anyhow::{Result, bail, ensure};
use auki_component_volume_monitor::demo::{RunOptions, run};

#[tokio::main]
async fn main() -> Result<()> {
    let mut options = RunOptions::default();
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--seconds" => {
                let seconds: u64 = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--seconds needs a value"))?
                    .parse()?;
                ensure!((1..=300).contains(&seconds), "--seconds must be 1..300");
                options.blocks = seconds * 100;
            }
            "--microphone" => options.microphone = true,
            "--help" | "-h" => {
                println!(
                    "Usage: auki-component-volume-monitor [--seconds 1..300] [--microphone]\n\nDefault: two synthetic peers over authenticated loopback TCP; no credentials or microphone needed.\n--microphone: capture the default microphone on A (build with --features microphone); B remains synthetic.\nData stays in memory. No playback, shared services, relay, or disk recording. Ctrl-C stops both peers."
                );
                return Ok(());
            }
            _ => bail!("unknown argument: {argument}; use --help"),
        }
    }
    eprintln!(
        "Local Component demo: A={}, B=synthetic. No shared services.",
        if options.microphone {
            "microphone"
        } else {
            "synthetic"
        }
    );
    let report = run(options).await?;
    for (label, local, remote) in [
        ("A", &report.a, &report.a_observed_b),
        ("B", &report.b, &report.b_observed_a),
    ] {
        println!("Peer {label}: {}", local.peer_id);
        println!(
            "  audio Buffer: {} blocks; volume Episode: {} observations, concluded={}",
            local.audio_entries, local.volume_observations, local.concluded
        );
        println!(
            "  remote: {} observations, {} missing, latest={:?} dBFS",
            remote.observations, remote.gap_entries, remote.latest_dbfs
        );
    }
    Ok(())
}
