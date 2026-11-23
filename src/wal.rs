use crate::{
    replay_record::{Record, unhex},
    rpc::Failure,
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    io::AsyncWriteExt,
    sync::{mpsc, oneshot},
};
const MAX_RECORD_BYTES: u64 = 4 * 1024 * 1024;
#[derive(Clone, Copy)]
pub struct StorageLimits {
    pub max_bytes: u64,
    pub max_files: u64,
}
struct Work {
    record: Record,
    reply: oneshot::Sender<Result<Record, Failure>>,
}
pub struct WalWriter {
    queue: mpsc::Sender<Work>,
}
impl WalWriter {
    pub fn start(
        root: PathBuf,
        key: SigningKey,
        capacity: usize,
        anchors: BTreeMap<u64, (u64, [u8; 32])>,
        limits: StorageLimits,
    ) -> Result<Self, crate::transport::Error> {
        if capacity == 0 {
            return Err("WAL queue capacity must be positive".into());
        }
        if limits.max_bytes == 0 || limits.max_files == 0 {
            return Err("WAL storage byte and file limits must be positive".into());
        }
        let (queue, mut receive) = mpsc::channel::<Work>(capacity);
        let key = Arc::new(key);
        tokio::spawn(async move {
            let mut anchors = anchors;
            while let Some(work) = receive.recv().await {
                let result = write(&root, key.clone(), work.record, &mut anchors, limits).await;
                let _closed = work.reply.send(result);
            }
        });
        Ok(Self { queue })
    }
    pub async fn append(&self, record: Record) -> Result<Record, Failure> {
        let (reply, result) = oneshot::channel();
        self.queue
            .send(Work { record, reply })
            .await
            .map_err(|_| Failure::UNAVAILABLE)?;
        result.await.map_err(|_| Failure::UNAVAILABLE)?
    }
}
async fn write(
    root: &Path,
    key: Arc<SigningKey>,
    mut record: Record,
    anchors: &mut BTreeMap<u64, (u64, [u8; 32])>,
    limits: StorageLimits,
) -> Result<Record, Failure> {
    let (last, previous) = anchors
        .get(&record.guild_battle_id)
        .copied()
        .unwrap_or((0, [0; 32]));
    let expected = last.checked_add(1).ok_or(Failure::CONFLICT)?;
    let key_for_signing = key.clone();
    record = tokio::task::spawn_blocking(move || {
        record.sign(&key_for_signing)?;
        Ok::<_, Failure>(record)
    })
    .await
    .map_err(|_| Failure::UNAVAILABLE)??;
    record.verify(&key.verifying_key(), expected, previous)?;
    persist(root, &record, limits).await?;
    anchors.insert(
        record.guild_battle_id,
        (record.event_index, unhex(&record.event_hash)?),
    );
    Ok(record)
}
async fn persist(root: &Path, record: &Record, limits: StorageLimits) -> Result<(), Failure> {
    let metadata = tokio::fs::symlink_metadata(root).await.map_err(io)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(Failure::CONFLICT);
    }
    let folder = root.join(format!(
        "guild_battle_{}_{}",
        record.guild_battle_id, record.instance_id
    ));
    tokio::fs::create_dir_all(&folder).await.map_err(io)?;
    if tokio::fs::symlink_metadata(&folder)
        .await
        .map_err(io)?
        .file_type()
        .is_symlink()
    {
        return Err(Failure::CONFLICT);
    }
    // Durable directory operations are required; unsupported filesystems must fail closed.
    sync_directory(root).await?;
    let target = folder.join(format!(
        "event_{}_{}.json",
        record.event_index, record.operation_id
    ));
    let bytes = serde_json::to_vec(record).map_err(|_| Failure::BAD_REQUEST)?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(Failure::BAD_REQUEST);
    }
    if tokio::fs::try_exists(&target).await.map_err(io)? {
        if tokio::fs::metadata(&target).await.map_err(io)?.len() > MAX_RECORD_BYTES {
            return Err(Failure::CONFLICT);
        }
        if tokio::fs::symlink_metadata(&target)
            .await
            .map_err(io)?
            .file_type()
            .is_symlink()
        {
            return Err(Failure::CONFLICT);
        }
        let existing = tokio::fs::read(&target).await.map_err(io)?;
        if existing != bytes {
            return Err(Failure::CONFLICT);
        }
        tokio::fs::File::open(&target)
            .await
            .map_err(io)?
            .sync_all()
            .await
            .map_err(io)?;
        return sync_directory(&folder).await;
    }
    check_storage(root, bytes.len() as u64, limits).await?;
    let temporary = folder.join(format!(
        ".event_{}_{}.tmp",
        record.event_index,
        uuid::Uuid::new_v4()
    ));
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .await
        .map_err(io)?;
    file.write_all(&bytes).await.map_err(io)?;
    file.sync_all().await.map_err(io)?;
    drop(file);
    tokio::fs::rename(&temporary, &target).await.map_err(io)?;
    sync_directory(&folder).await
}

// The instance storage root must have one writer. Count unfinished temporary files too:
// failed persistence must never make quota usage disappear before recovery verifies it.
async fn check_storage(root: &Path, additional: u64, limits: StorageLimits) -> Result<(), Failure> {
    let mut pending = vec![root.to_path_buf()];
    let mut used_bytes = 0_u64;
    let mut used_files = 0_u64;
    while let Some(folder) = pending.pop() {
        let mut entries = tokio::fs::read_dir(folder).await.map_err(io)?;
        while let Some(entry) = entries.next_entry().await.map_err(io)? {
            let kind = entry.file_type().await.map_err(io)?;
            if kind.is_symlink() {
                return Err(Failure::CONFLICT);
            }
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !kind.is_file() {
                return Err(Failure::CONFLICT);
            }
            let size = entry.metadata().await.map_err(io)?.len();
            used_bytes = used_bytes.checked_add(size).ok_or(Failure::UNAVAILABLE)?;
            used_files = used_files.checked_add(1).ok_or(Failure::UNAVAILABLE)?;
            if used_bytes > limits.max_bytes || used_files > limits.max_files {
                return Err(Failure::UNAVAILABLE);
            }
        }
    }
    // Projected usage = all existing files + this one immutable record (temp is renamed).
    if additional > limits.max_bytes.saturating_sub(used_bytes) || used_files == limits.max_files {
        return Err(Failure::UNAVAILABLE);
    }
    Ok(())
}
#[cfg(unix)]
async fn sync_directory(path: &Path) -> Result<(), Failure> {
    tokio::fs::File::open(path)
        .await
        .map_err(io)?
        .sync_all()
        .await
        .map_err(io)
}
#[cfg(not(unix))]
async fn sync_directory(_path: &Path) -> Result<(), Failure> {
    Err(Failure::UNAVAILABLE)
}
fn io(_: std::io::Error) -> Failure {
    Failure::UNAVAILABLE
}
/// Verification only: recovery callers must also compare DB operations and replay-file contents before deletion.
pub async fn verify_directory(
    folder: &Path,
    instance: &str,
    battle: u64,
    keys: &BTreeMap<String, VerifyingKey>,
    anchor: (u64, [u8; 32]),
) -> Result<Vec<Record>, Failure> {
    let mut entries = tokio::fs::read_dir(folder).await.map_err(io)?;
    let mut records = BTreeMap::new();
    while let Some(entry) = entries.next_entry().await.map_err(io)? {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(Failure::CONFLICT);
        };
        if !name.starts_with("event_") || !name.ends_with(".json") {
            continue;
        }
        let metadata = entry.metadata().await.map_err(io)?;
        if entry.file_type().await.map_err(io)?.is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_RECORD_BYTES
        {
            return Err(Failure::CONFLICT);
        }
        let bytes = tokio::fs::read(entry.path()).await.map_err(io)?;
        let record: Record = serde_json::from_slice(&bytes).map_err(|_| Failure::CONFLICT)?;
        if record.instance_id != instance
            || record.guild_battle_id != battle
            || name != format!("event_{}_{}.json", record.event_index, record.operation_id)
        {
            return Err(Failure::CONFLICT);
        }
        if records.insert(record.event_index, record).is_some() {
            return Err(Failure::CONFLICT);
        }
    }
    let (mut index, mut previous) = anchor;
    for record in records.values() {
        index = index.checked_add(1).ok_or(Failure::CONFLICT)?;
        record.verify(
            keys.get(&record.signing_key_id).ok_or(Failure::CONFLICT)?,
            index,
            previous,
        )?;
        previous = unhex(&record.event_hash)?;
    }
    Ok(records.into_values().collect())
}
