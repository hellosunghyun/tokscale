//! Separate reported Grok billing snapshots. Never a UnifiedMessage source.
use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Utc};
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

mod cache;
#[cfg(unix)]
mod transport;

const SOURCE: &str = "grok-dashboard-billing-v2";
#[cfg(unix)]
const LIMIT: usize = 16 * 1024 * 1024;
const METERS: [&str; 4] = [
    "prompt-text-tokens",
    "completion-text-tokens",
    "cached-prompt-text-tokens",
    "cache-write-text-tokens",
];

#[derive(Clone, Copy, ValueEnum)]
pub enum Transport {
    Aside,
}

#[derive(Args)]
pub struct RangeArgs {
    #[arg(
        long,
        requires = "end",
        help = "Inclusive RFC3339 UTC instant ending in Z"
    )]
    start: Option<String>,
    #[arg(
        long,
        requires = "start",
        help = "Exclusive RFC3339 UTC instant ending in Z"
    )]
    end: Option<String>,
}

#[derive(Subcommand)]
pub enum Command {
    #[command(
        about = "Experimentally collect separate Grok Bot source records through local Aside (macOS)"
    )]
    Sync {
        #[arg(
            long,
            value_enum,
            help = "Explicit browser transport; requires logged-in local Aside and Python 3"
        )]
        transport: Transport,
        #[command(flatten)]
        range: RangeArgs,
        #[arg(long, default_value_t = 128, value_parser = clap::value_parser!(u32).range(2..=512))]
        max_pages: u32,
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u32).range(10..=600))]
        timeout: u32,
        #[arg(
            long,
            help = "Emit safe metadata only; errors may include operational page counts, never records, token totals, owners, dates or paths"
        )]
        redacted: bool,
    },
    #[command(
        about = "Read the latest private source snapshot (not current browser account or canonical reports)"
    )]
    History {
        #[arg(long, conflicts_with = "redacted")]
        json: bool,
        #[arg(long)]
        redacted: bool,
    },
}

// No Debug: private fields never enter diagnostics. Serialize only to private cache
// or an explicitly requested local history view, never subprocess logs.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Range {
    start_time: String,
    end_time: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Principal {
    #[serde(rename = "type")]
    kind: String,
    id: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    billing_item_id: String,
    event_time: String,
    product: String,
    charge_category: String,
    usage_type: String,
    principal: Principal,
    has_token_meters: bool,
    usage: BTreeMap<String, String>,
    total_tokens: String,
    #[serde(deserialize_with = "unknown_value")]
    model: Option<String>,
    #[serde(deserialize_with = "unknown_value")]
    cost: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Snapshot {
    version: u32,
    source: String,
    transport: String,
    scope: String,
    owner: String,
    owner_rechecked: bool,
    range: Range,
    api_pagination_complete: bool,
    coverage: String,
    retention_known: bool,
    corrections_known: bool,
    collected_at: String,
    records: Vec<Record>,
}

fn unknown_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    if value.is_null() {
        Ok(None)
    } else {
        Err(serde::de::Error::custom("unknown field must be null"))
    }
}

fn instant(value: &str) -> Result<DateTime<Utc>> {
    if value.len() > 40 || !value.ends_with('Z') {
        bail!("grok_history_invalid_range");
    }
    DateTime::parse_from_rfc3339(value)
        .map(|v| v.with_timezone(&Utc))
        .map_err(|_| anyhow!("grok_history_invalid_range"))
}
fn range_valid(range: &Range) -> Result<()> {
    if instant(&range.start_time)? >= instant(&range.end_time)? {
        bail!("grok_history_invalid_range");
    }
    Ok(())
}
fn text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && value.trim() == value
}
fn unsigned(value: &str) -> Result<u128> {
    if value.is_empty() || value.len() > 39 || !value.bytes().all(|v| v.is_ascii_digit()) {
        bail!("grok_history_invalid_meter");
    }
    value
        .parse()
        .map_err(|_| anyhow!("grok_history_meter_overflow"))
}
fn validate(snapshot: &Snapshot) -> Result<()> {
    range_valid(&snapshot.range)?;
    if snapshot.version != 1
        || snapshot.source != SOURCE
        || snapshot.transport != "aside-local"
        || snapshot.scope != "user"
        || !text(&snapshot.owner)
        || !snapshot.owner_rechecked
        || !snapshot.api_pagination_complete
        || snapshot.coverage != "requested_range_only"
        || snapshot.retention_known
        || snapshot.corrections_known
        || snapshot.records.len() > 100000
    {
        bail!("grok_history_snapshot_invalid");
    }
    instant(&snapshot.collected_at)?;
    let start = instant(&snapshot.range.start_time)?;
    let end = instant(&snapshot.range.end_time)?;
    let mut ids = HashSet::new();
    for row in &snapshot.records {
        if !text(&row.billing_item_id)
            || !ids.insert(&row.billing_item_id)
            || row.product != "grok-bot"
            || row.principal.kind != "user"
            || row.principal.id != snapshot.owner
            || !text(&row.charge_category)
            || !text(&row.usage_type)
            || !row.has_token_meters
            || row.model.is_some()
            || row.cost.is_some()
            || row.usage.len() != METERS.len()
        {
            bail!("grok_history_record_invalid");
        }
        let time = instant(&row.event_time)?;
        if time < start || time >= end {
            bail!("grok_history_event_outside_range");
        }
        let mut sum = 0u128;
        for key in METERS {
            sum = sum
                .checked_add(unsigned(
                    row.usage
                        .get(key)
                        .ok_or_else(|| anyhow!("grok_history_invalid_meter"))?,
                )?)
                .ok_or_else(|| anyhow!("grok_history_meter_overflow"))?;
        }
        if sum != unsigned(&row.total_tokens)? {
            bail!("grok_history_total_mismatch");
        }
    }
    Ok(())
}
#[cfg(unix)]
fn decode(bytes: &[u8]) -> Result<Snapshot> {
    if bytes.len() > LIMIT {
        bail!("grok_history_byte_limit");
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| anyhow!("grok_history_snapshot_invalid"))?;
    // Missing unknown fields must not be silently interpreted as known zero/cost.
    if value
        .get("records")
        .and_then(|v| v.as_array())
        .is_some_and(|rows| {
            rows.iter().any(|row| {
                row.get("model") != Some(&serde_json::Value::Null)
                    || row.get("cost") != Some(&serde_json::Value::Null)
            })
        })
    {
        bail!("grok_history_record_invalid");
    }
    let snapshot: Snapshot =
        serde_json::from_value(value).map_err(|_| anyhow!("grok_history_snapshot_invalid"))?;
    validate(&snapshot)?;
    Ok(snapshot)
}
fn directory() -> Result<PathBuf> {
    Ok(std::env::var_os("TOKSCALE_GROK_BOT_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::paths::get_cache_dir().join("grok-bot-browser")))
}
fn report(snapshot: &Snapshot, saved: bool, redacted: bool, json: bool) -> Result<()> {
    if redacted {
        println!(
            "{}",
            serde_json::json!({"source":SOURCE,"transport":"aside-local","api_pagination_complete":true,"cache_saved":saved,"cache_read_ok":!saved,"records_present":!snapshot.records.is_empty(),"coverage":"requested_range_only","model_known":false,"cost_known":false})
        );
    } else if json {
        println!(
            "{}",
            serde_json::to_string_pretty(snapshot)
                .map_err(|_| anyhow!("grok_history_output_failed"))?
        );
    } else {
        println!("Grok Bot browser source snapshot (not canonical token reports)");
        println!(
            "Requested UTC range: {} to {} (exclusive)",
            snapshot.range.start_time, snapshot.range.end_time
        );
        println!(
            "Reported records: {}; model/cost unknown",
            snapshot.records.len()
        );
        println!("API pagination complete; retention/corrections unknown. Cached owner at collection time.");
        if snapshot.records.is_empty() {
            println!("No records returned; this does not establish zero usage.");
        }
    }
    Ok(())
}

#[cfg(unix)]
fn sync_with(
    dir: &Path,
    requested: Option<&Range>,
    collect: impl FnOnce() -> Result<Vec<u8>>,
) -> Result<Snapshot> {
    cache::prepare(dir)?;
    let bytes = collect()?;
    let snapshot = decode(&bytes)?;
    if requested.is_some_and(|range| {
        range.start_time != snapshot.range.start_time || range.end_time != snapshot.range.end_time
    }) {
        bail!("grok_history_range_drift");
    }
    cache::save(dir, &snapshot)?;
    Ok(snapshot)
}

pub fn run(command: Command, offline: bool) -> Result<()> {
    match command {
        Command::Sync {
            transport: Transport::Aside,
            range,
            max_pages,
            timeout,
            redacted,
        } => {
            if offline {
                bail!("grok_history_sync_requires_browser_network");
            }
            if !cfg!(target_os = "macos") {
                bail!("grok_history_aside_requires_macos");
            }
            let requested = match (range.start, range.end) {
                (None, None) => None,
                (Some(start_time), Some(end_time)) => {
                    let range = Range {
                        start_time,
                        end_time,
                    };
                    range_valid(&range)?;
                    Some(range)
                }
                _ => bail!("grok_history_invalid_range"),
            };
            let dir = directory()?;
            cache::prepare(&dir)?;
            #[cfg(unix)]
            {
                let snapshot = sync_with(&dir, requested.as_ref(), || {
                    transport::collect(requested.as_ref(), max_pages, timeout)
                })?;
                report(&snapshot, true, redacted, false)
            }
            #[cfg(not(unix))]
            {
                let _ = (requested, max_pages, timeout, redacted, dir);
                bail!("grok_history_aside_requires_macos")
            }
        }
        Command::History { json, redacted } => {
            let snapshot = cache::latest(&directory()?)?;
            report(&snapshot, false, redacted, json)
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
