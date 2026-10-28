#![cfg(unix)]
use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;

#[test]
fn explicit_transport_and_offline_guard_do_not_initialize_scanner_settings() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let data = root.join("private");
    let config = root.join("scanner-config");
    let result = cargo_bin_cmd!("tokscale")
        .args([
            "--no-spinner",
            "--offline",
            "grok-bot",
            "sync",
            "--transport",
            "aside",
            "--redacted",
        ])
        .env("TOKSCALE_GROK_BOT_DATA_DIR", &data)
        .env("TOKSCALE_CONFIG_DIR", &config)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr)
        .contains("grok_history_sync_requires_browser_network"));
    assert!(!data.exists() && !config.exists());
    let result = cargo_bin_cmd!("tokscale")
        .args(["--no-spinner", "grok-bot", "sync"])
        .env("TOKSCALE_GROK_BOT_DATA_DIR", &data)
        .env("TOKSCALE_CONFIG_DIR", &config)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!data.exists() && !config.exists());
}

#[test]
fn real_history_command_reads_only_private_cache_with_redacted_debug_output() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let data = root.join("private");
    let config = root.join("scanner-config");
    fs::create_dir(&data).unwrap();
    fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
    let snapshot = json!({"version":1,"source":"grok-dashboard-billing-v2","transport":"aside-local","scope":"user","owner":"private-canary-owner","ownerRechecked":true,"range":{"startTime":"2020-01-01T00:00:00Z","endTime":"2020-02-01T00:00:00Z"},"apiPaginationComplete":true,"coverage":"requested_range_only","retentionKnown":false,"correctionsKnown":false,"collectedAt":"2020-02-01T00:00:00Z","records":[]});
    let path = data.join("history.json");
    fs::write(
        &path,
        json!({"version":1,"snapshots":[snapshot]}).to_string(),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let result = cargo_bin_cmd!("tokscale")
        .args([
            "--no-spinner",
            "--debug",
            "--offline",
            "grok-bot",
            "history",
            "--redacted",
        ])
        .env("TOKSCALE_GROK_BOT_DATA_DIR", &data)
        .env("TOKSCALE_CONFIG_DIR", &config)
        .output()
        .unwrap();
    assert!(result.status.success());
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(
        value["cache_read_ok"] == true
            && value["api_pagination_complete"] == true
            && value["records_present"] == false
    );
    let output = String::from_utf8_lossy(&result.stdout).to_string()
        + &String::from_utf8_lossy(&result.stderr);
    assert!(
        !output.contains("private-canary")
            && !output.contains("2020-01")
            && !output.contains(root.to_str().unwrap())
    );
    assert!(!config.exists());
}
