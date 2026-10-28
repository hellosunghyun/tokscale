use super::*;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    version: u32,
    snapshots: Vec<Snapshot>,
}
const STORE_LIMIT: usize = 64 * 1024 * 1024;

fn safe_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path.components().any(|part| {
            matches!(
                part,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        bail!("grok_history_private_directory_invalid");
    }
    for parent in path.ancestors() {
        if fs::symlink_metadata(parent.join(".git")).is_ok() {
            bail!("grok_history_cache_in_worktree");
        }
        if let Ok(info) = fs::symlink_metadata(parent) {
            if info.file_type().is_symlink() {
                bail!("grok_history_private_directory_invalid");
            }
        }
    }
    Ok(())
}
#[cfg(unix)]
fn private(info: &fs::Metadata, directory: bool) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    if info.uid() != unsafe { libc::geteuid() }
        || info.mode() & 0o7777 != if directory { 0o700 } else { 0o600 }
        || if directory {
            !info.is_dir()
        } else {
            !info.is_file() || info.nlink() != 1
        }
    {
        bail!("grok_history_private_permissions_invalid");
    }
    Ok(())
}
#[cfg(not(unix))]
fn private(_: &fs::Metadata, _: bool) -> Result<()> {
    bail!("grok_history_private_storage_requires_unix")
}

pub(super) fn prepare(dir: &Path) -> Result<()> {
    safe_path(dir)?;
    if !dir.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .mode(0o700)
                .recursive(true)
                .create(dir)
                .map_err(|_| anyhow!("grok_history_cache_unavailable"))?;
        }
        #[cfg(not(unix))]
        {
            bail!("grok_history_private_storage_requires_unix");
        }
    }
    let info = fs::symlink_metadata(dir).map_err(|_| anyhow!("grok_history_cache_unavailable"))?;
    private(&info, true)
}
fn open(path: &Path, create: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(create).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|_| anyhow!("grok_history_cache_unavailable"))?;
    private(
        &file
            .metadata()
            .map_err(|_| anyhow!("grok_history_cache_unavailable"))?,
        false,
    )?;
    Ok(file)
}
fn read(dir: &Path) -> Result<Store> {
    let path = dir.join("history.json");
    if matches!(fs::symlink_metadata(&path),Err(error) if error.kind()==std::io::ErrorKind::NotFound)
    {
        return Ok(Store {
            version: 1,
            snapshots: vec![],
        });
    }
    let file = open(&path, false)?;
    let mut bytes = Vec::new();
    file.take((STORE_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow!("grok_history_cache_unavailable"))?;
    if bytes.len() > STORE_LIMIT {
        bail!("grok_history_cache_limit");
    }
    let store: Store =
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("grok_history_cache_invalid"))?;
    if store.version != 1 {
        bail!("grok_history_cache_invalid");
    }
    for snapshot in &store.snapshots {
        validate(snapshot)?;
    }
    Ok(store)
}
pub(super) fn save(dir: &Path, snapshot: &Snapshot) -> Result<()> {
    validate(snapshot)?;
    prepare(dir)?;
    let lock = open(&dir.join("history.lock"), true)?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| anyhow!("grok_history_cache_unavailable"))?;
    let mut store = read(dir)?;
    if snapshot.records.is_empty()
        && store.snapshots.iter().any(|old| {
            old.owner == snapshot.owner
                && old.range.start_time == snapshot.range.start_time
                && old.range.end_time == snapshot.range.end_time
                && !old.records.is_empty()
        })
    {
        bail!("grok_history_unattested_empty_preserved");
    }
    // Immutable owner/range versions share one atomic transaction; no public ID
    // filenames, cross-account merge, or independently updated index exists.
    let copied: Snapshot = serde_json::from_slice(
        &serde_json::to_vec(snapshot).map_err(|_| anyhow!("grok_history_cache_invalid"))?,
    )
    .map_err(|_| anyhow!("grok_history_cache_invalid"))?;
    store.snapshots.push(copied);
    let bytes = serde_json::to_vec(&store).map_err(|_| anyhow!("grok_history_cache_invalid"))?;
    if bytes.len() > STORE_LIMIT {
        bail!("grok_history_cache_limit");
    }
    let temp = dir.join(format!(".history-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options
            .open(&temp)
            .map_err(|_| anyhow!("grok_history_cache_unavailable"))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| anyhow!("grok_history_cache_unavailable"))?;
        fs::rename(&temp, dir.join("history.json"))
            .map_err(|_| anyhow!("grok_history_cache_unavailable"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
pub(super) fn latest(dir: &Path) -> Result<Snapshot> {
    prepare(dir)?;
    let store = read(dir)?;
    store
        .snapshots
        .into_iter()
        .last()
        .ok_or_else(|| anyhow!("grok_history_no_snapshot"))
}
