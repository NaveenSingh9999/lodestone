//! RCON metrics collector (spec 4.3 depth).
//!
//! Polls a running server's RCON interface and produces `MetricsSample`
//! values. The "list" command yields the current player count; other
//! fields (cpu, rss, tps, mspt) are left as None since they require
//! server-specific plugins or process introspection.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::events::MetricsSample;
use crate::rcon::{RconClient, RconConfig, RconError};

pub type MetricsResult<T> = Result<T, RconError>;

/// Collects metrics from a server over RCON.
pub struct MetricsCollector {
    config: RconConfig,
    instance_id: String,
}

impl MetricsCollector {
    pub fn new(config: RconConfig, instance_id: impl Into<String>) -> Self {
        Self {
            config,
            instance_id: instance_id.into(),
        }
    }

    /// Connect, authenticate, run "list", and return a metrics sample.
    pub async fn poll(&self) -> MetricsResult<MetricsSample> {
        let mut client = RconClient::connect(&self.config).await?;
        let output = client.execute("list").await?;
        let players_online = parse_list_output(&output);
        Ok(MetricsSample {
            instance_id: self.instance_id.clone(),
            timestamp_ms: now_ms(),
            cpu_percent: None,
            rss_bytes: None,
            tps: None,
            mspt: None,
            players_online,
        })
    }
}

/// Parse the "list" command output: "There are N of M players online"
/// or "There are no players online" or an empty/unknown response.
fn parse_list_output(output: &str) -> Option<u32> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }
    // "There are 3 of 20 players online"
    let rest = trimmed.strip_prefix("There are ").or(Some(trimmed))?;
    let (first, _rest) = rest.split_once(' ')?;
    let n: u32 = first.parse().ok()?;
    Some(n)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_list_with_players() {
        assert_eq!(
            parse_list_output("There are 3 of 20 players online"),
            Some(3)
        );
        assert_eq!(
            parse_list_output("There are 0 of 20 players online"),
            Some(0)
        );
        assert_eq!(
            parse_list_output("There are 12 of 100 players online"),
            Some(12)
        );
    }

    #[test]
    fn parse_list_no_players() {
        assert_eq!(parse_list_output("There are no players online"), None);
        assert_eq!(parse_list_output(""), None);
        assert_eq!(parse_list_output("   "), None);
        assert_eq!(parse_list_output("Unknown command"), None);
    }
}
