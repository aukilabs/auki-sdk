use crate::flash::FlashMode;
use std::collections::BTreeMap;

const MAX_REPORTS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct TickReport {
    pub peer_id: String,
    pub peer_suffix: String,
    pub tick_id: i64,
    pub mode: FlashMode,
    pub utc_observed_ns: i64,
    pub biased_utc_observed_ns: i64,
    pub domain_observed_ns: Option<i64>,
    pub simulated_utc_offset_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PeerTickStats {
    pub peer_suffix: String,
    pub utc_latest_delta_ms: Option<f64>,
    pub utc_p50_delta_ms: Option<f64>,
    pub utc_p95_delta_ms: Option<f64>,
    pub utc_max_delta_ms: Option<f64>,
    pub domain_latest_delta_ms: Option<f64>,
    pub domain_p50_delta_ms: Option<f64>,
    pub domain_p95_delta_ms: Option<f64>,
    pub domain_max_delta_ms: Option<f64>,
    pub improvement_ratio: Option<f64>,
    pub samples: usize,
}

#[derive(Debug, Default, Clone)]
pub struct TickReportStore {
    local: Vec<TickReport>,
    remote: Vec<TickReport>,
}

impl TickReportStore {
    pub fn record_local(&mut self, report: TickReport) {
        push_bounded(&mut self.local, report);
    }

    pub fn record_remote(&mut self, report: TickReport) {
        push_bounded(&mut self.remote, report);
    }

    pub fn peer_stats(&self) -> Vec<PeerTickStats> {
        let mut by_peer: BTreeMap<&str, Vec<&TickReport>> = BTreeMap::new();
        for report in &self.remote {
            by_peer.entry(&report.peer_id).or_default().push(report);
        }

        let mut stats = Vec::new();
        for (_peer_id, reports) in by_peer {
            let utc_deltas = deltas_for_mode(&self.local, &reports, FlashMode::Utc);
            let domain_deltas = deltas_for_mode(&self.local, &reports, FlashMode::Domain);
            if utc_deltas.is_empty() && domain_deltas.is_empty() {
                continue;
            }

            let utc_latest = utc_deltas.last().copied();
            let domain_latest = domain_deltas.last().copied();
            let improvement_ratio = match (utc_latest, domain_latest) {
                (Some(utc), Some(domain)) if domain > 0.0 => Some(round1(utc / domain)),
                _ => None,
            };
            let peer_suffix = reports
                .last()
                .map(|report| report.peer_suffix.clone())
                .unwrap_or_default();

            stats.push(PeerTickStats {
                peer_suffix,
                utc_latest_delta_ms: utc_latest,
                utc_p50_delta_ms: percentile(&utc_deltas, 0.50),
                utc_p95_delta_ms: percentile(&utc_deltas, 0.95),
                utc_max_delta_ms: max(&utc_deltas),
                domain_latest_delta_ms: domain_latest,
                domain_p50_delta_ms: percentile(&domain_deltas, 0.50),
                domain_p95_delta_ms: percentile(&domain_deltas, 0.95),
                domain_max_delta_ms: max(&domain_deltas),
                improvement_ratio,
                samples: utc_deltas.len().max(domain_deltas.len()),
            });
        }
        stats
    }
}

fn push_bounded(reports: &mut Vec<TickReport>, report: TickReport) {
    reports.push(report);
    if reports.len() > MAX_REPORTS {
        reports.remove(0);
    }
}

fn deltas_for_mode(local: &[TickReport], remote: &[&TickReport], mode: FlashMode) -> Vec<f64> {
    let mut deltas = Vec::new();
    for remote_report in remote.iter().filter(|report| report.mode == mode) {
        let Some(local_report) = local
            .iter()
            .rev()
            .find(|report| report.mode == mode && report.tick_id == remote_report.tick_id)
        else {
            continue;
        };
        let delta_ns = match mode {
            FlashMode::Utc => {
                remote_report.biased_utc_observed_ns - local_report.biased_utc_observed_ns
            }
            FlashMode::Domain => {
                let (Some(remote_domain), Some(local_domain)) = (
                    remote_report.domain_observed_ns,
                    local_report.domain_observed_ns,
                ) else {
                    continue;
                };
                remote_domain - local_domain
            }
        };
        deltas.push(round1(delta_ns.unsigned_abs() as f64 / 1_000_000.0));
    }
    deltas
}

fn percentile(values: &[f64], percentile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((sorted.len() - 1) as f64 * percentile).ceil() as usize;
    Some(sorted[index])
}

fn max(values: &[f64]) -> Option<f64> {
    values.iter().copied().max_by(f64::total_cmp)
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(peer_id: &str, mode: FlashMode, tick_id: i64, observed_ns: i64) -> TickReport {
        TickReport {
            peer_id: peer_id.into(),
            peer_suffix: format!("...{peer_id}"),
            tick_id,
            mode,
            utc_observed_ns: observed_ns,
            biased_utc_observed_ns: observed_ns,
            domain_observed_ns: if mode == FlashMode::Domain {
                Some(observed_ns)
            } else {
                None
            },
            simulated_utc_offset_ms: 0,
        }
    }

    #[test]
    fn sync_stats_match_reports_by_mode_and_tick_id() {
        let mut store = TickReportStore::default();
        store.record_local(report("local", FlashMode::Utc, 10, 30_000_000_000));
        store.record_remote(report("peer", FlashMode::Utc, 10, 30_250_000_000));
        store.record_local(report("local", FlashMode::Domain, 11, 33_000_000_000));
        store.record_remote(report("peer", FlashMode::Domain, 11, 33_006_000_000));

        let stats = store.peer_stats();

        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].peer_suffix, "...peer");
        assert_eq!(stats[0].utc_latest_delta_ms, Some(250.0));
        assert_eq!(stats[0].domain_latest_delta_ms, Some(6.0));
        assert_eq!(stats[0].improvement_ratio, Some(41.7));
    }

    #[test]
    fn sync_stats_ignore_reports_from_different_ticks() {
        let mut store = TickReportStore::default();
        store.record_local(report("local", FlashMode::Utc, 10, 30_000_000_000));
        store.record_remote(report("peer", FlashMode::Utc, 11, 33_000_000_000));

        assert!(store.peer_stats().is_empty());
    }
}
