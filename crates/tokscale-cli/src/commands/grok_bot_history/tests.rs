use super::*;
use serde_json::{json, Value};

fn fixture() -> Value {
    json!({"version":1,"source":SOURCE,"transport":"aside-local","scope":"user","owner":"synthetic-owner","ownerRechecked":true,"range":{"startTime":"2020-01-01T00:00:00Z","endTime":"2020-02-01T00:00:00Z"},"apiPaginationComplete":true,"coverage":"requested_range_only","retentionKnown":false,"correctionsKnown":false,"collectedAt":"2020-02-01T00:00:00Z","records":[{"billingItemId":"synthetic-item","eventTime":"2020-01-02T01:02:03.004Z","product":"grok-bot","chargeCategory":"included","usageType":"included","principal":{"type":"user","id":"synthetic-owner"},"hasTokenMeters":true,"usage":{"prompt-text-tokens":"9007199254740993","completion-text-tokens":"2","cached-prompt-text-tokens":"3","cache-write-text-tokens":"4"},"totalTokens":"9007199254741002","model":null,"cost":null}]})
}
fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
fn private_temp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn exact_reported_strings_and_unknown_values_survive_connected_cache() {
    let temp = private_temp();
    let dir = temp.path().canonicalize().unwrap().join("private");
    let snapshot = sync_with(&dir, None, || Ok(bytes(&fixture()))).unwrap();
    assert!(snapshot.records[0].usage[METERS[0]] == "9007199254740993");
    let saved = cache::latest(&dir).unwrap();
    assert!(
        saved.records[0].total_tokens == "9007199254741002"
            && saved.records[0].model.is_none()
            && saved.records[0].cost.is_none()
    );
    let raw: Value =
        serde_json::from_slice(&std::fs::read(dir.join("history.json")).unwrap()).unwrap();
    assert!(raw["snapshots"][0]["records"][0]["model"].is_null());
}

#[test]
fn malformed_missing_numeric_and_scope_fields_keep_old_cache_bytes() {
    let temp = private_temp();
    let dir = temp.path().canonicalize().unwrap().join("private");
    sync_with(&dir, None, || Ok(bytes(&fixture()))).unwrap();
    let before = std::fs::read(dir.join("history.json")).unwrap();
    let mut cases = Vec::new();
    for bad in [
        json!(-1),
        json!(1),
        json!(null),
        json!("-1"),
        json!("1.5"),
        json!("1e3"),
        json!(" 1"),
        json!(""),
        json!("340282366920938463463374607431768211456"),
    ] {
        let mut value = fixture();
        value["records"][0]["usage"][METERS[0]] = bad;
        cases.push(value);
    }
    for key in [
        "totalTokens",
        "hasTokenMeters",
        "principal",
        "eventTime",
        "model",
        "cost",
    ] {
        let mut v = fixture();
        v["records"][0].as_object_mut().unwrap().remove(key);
        cases.push(v);
    }
    for (key, bad) in [
        ("ownerRechecked", json!(false)),
        ("apiPaginationComplete", json!(false)),
        ("scope", json!("team")),
        ("owner", json!("other-owner")),
    ] {
        let mut v = fixture();
        v[key] = bad;
        cases.push(v);
    }
    for (key, bad) in [
        ("product", json!("grok-build")),
        ("model", json!("invented-model")),
        ("cost", json!("0")),
        ("totalTokens", json!("1")),
        ("eventTime", json!("2020-02-01T00:00:00Z")),
    ] {
        let mut v = fixture();
        v["records"][0][key] = bad;
        cases.push(v);
    }
    let mut missing = fixture();
    missing["records"][0]["usage"]
        .as_object_mut()
        .unwrap()
        .remove(METERS[0]);
    cases.push(missing);
    let mut duplicate = fixture();
    let record = duplicate["records"][0].clone();
    duplicate["records"].as_array_mut().unwrap().push(record);
    cases.push(duplicate);
    for value in cases {
        assert!(sync_with(&dir, None, || Ok(bytes(&value))).is_err());
        assert!(std::fs::read(dir.join("history.json")).unwrap() == before);
    }
    for class in [
        "http_auth",
        "http_rate_limit",
        "response_not_json",
        "collection_deadline",
        "byte_limit",
        "invalid_cursor",
        "scope_drift",
        "owner_changed",
    ] {
        assert!(sync_with(&dir, None, || Err(anyhow!(class))).is_err());
        assert!(std::fs::read(dir.join("history.json")).unwrap() == before);
    }
}

#[test]
fn empty_does_not_erase_populated_snapshot_and_owners_ranges_do_not_merge() {
    let temp = private_temp();
    let dir = temp.path().canonicalize().unwrap().join("private");
    sync_with(&dir, None, || Ok(bytes(&fixture()))).unwrap();
    let before = std::fs::read(dir.join("history.json")).unwrap();
    let mut empty = fixture();
    empty["records"] = json!([]);
    assert_eq!(
        sync_with(&dir, None, || Ok(bytes(&empty)))
            .err()
            .unwrap()
            .to_string(),
        "grok_history_unattested_empty_preserved"
    );
    assert!(std::fs::read(dir.join("history.json")).unwrap() == before);
    empty["owner"] = json!("another-owner");
    let snapshot = sync_with(&dir, None, || Ok(bytes(&empty))).unwrap();
    assert!(snapshot.records.is_empty());
    empty["range"]["startTime"] = json!("2020-01-03T00:00:00Z");
    sync_with(&dir, None, || Ok(bytes(&empty))).unwrap();
    let raw: Value =
        serde_json::from_slice(&std::fs::read(dir.join("history.json")).unwrap()).unwrap();
    assert!(raw["snapshots"].as_array().unwrap().len() == 3);
    assert!(raw["snapshots"][0]["records"].as_array().unwrap().len() == 1);
    assert!(raw["snapshots"][1]["owner"] != raw["snapshots"][0]["owner"]);
}

#[test]
fn range_validation_and_u128_arithmetic_are_checked_not_rounded() {
    assert!(unsigned("340282366920938463463374607431768211455").unwrap() == u128::MAX);
    let mut value = fixture();
    value["records"][0]["usage"][METERS[0]] = json!(u128::MAX.to_string());
    assert!(decode(&bytes(&value)).err().unwrap().to_string() == "grok_history_meter_overflow");
    for bad in [
        "not-date",
        "2020-02-30T00:00:00Z",
        "2020-01-01T00:00:00+00:00",
    ] {
        assert!(instant(bad).is_err());
    }
    let temp = private_temp();
    let dir = temp.path().canonicalize().unwrap().join("private");
    let range = Range {
        start_time: "2020-01-02T00:00:00Z".into(),
        end_time: "2020-02-01T00:00:00Z".into(),
    };
    assert!(sync_with(&dir, Some(&range), || Ok(bytes(&fixture()))).is_err());
    assert!(!dir.join("history.json").exists());
}

#[cfg(unix)]
#[test]
fn private_cache_rejects_worktrees_symlinks_modes_and_preserves_corruption() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let temp = private_temp();
    let root = temp.path().canonicalize().unwrap();
    let dir = root.join("private");
    sync_with(&dir, None, || Ok(bytes(&fixture()))).unwrap();
    assert!(std::fs::metadata(&dir).unwrap().mode() & 0o777 == 0o700);
    assert!(std::fs::metadata(dir.join("history.json")).unwrap().mode() & 0o777 == 0o600);
    let repo = root.join("repo");
    std::fs::create_dir(&repo).unwrap();
    std::fs::write(repo.join(".git"), b"worktree").unwrap();
    assert!(cache::prepare(&repo.join("data")).is_err());
    let alias = root.join("alias");
    std::os::unix::fs::symlink(&dir, &alias).unwrap();
    assert!(cache::prepare(&alias).is_err());
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(cache::latest(&dir).is_err());
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(dir.join("history.json"), b"private-canary-invalid").unwrap();
    let error = sync_with(&dir, None, || Ok(bytes(&fixture())))
        .err()
        .unwrap();
    assert!(!format!("{error:?}").contains("private-canary"));
    assert!(std::fs::read(dir.join("history.json")).unwrap() == b"private-canary-invalid");
}

#[cfg(unix)]
#[test]
fn safe_http_failures_preserve_empty_and_existing_cache() {
    for (code, status) in [
        ("http_auth", 401),
        ("http_permission", 403),
        ("http_rate_limit", 429),
        ("http_server", 500),
        ("http_server", 503),
    ] {
        let temp = private_temp();
        let dir = temp.path().canonicalize().unwrap().join("private");
        let diagnostic =
            json!({"http_status":status,"phase":"walk","successful_pages":2,"elapsed_ms":1250});
        let error = sync_with(&dir, None, || {
            Err(transport::control_failure(code, Some(&diagnostic)))
        })
        .err()
        .unwrap();
        assert!(error.to_string().contains(&format!("http_status={status}")));
        assert!(!dir.join("history.json").exists());
        assert!(!dir.join("history.lock").exists());
        sync_with(&dir, None, || Ok(bytes(&fixture()))).unwrap();
        let before = std::fs::read(dir.join("history.json")).unwrap();
        assert!(sync_with(&dir, None, || Err(transport::control_failure(
            code,
            Some(&diagnostic)
        )))
        .is_err());
        assert!(before == std::fs::read(dir.join("history.json")).unwrap());
    }
}
