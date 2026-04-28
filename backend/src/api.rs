use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::Response,
    Json,
};
use axum_extra::extract::Multipart;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::fs::File;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::xmap::{
    XmapCache, XmapFileSet, parse_xmap_disk, parse_refinefinal_cached,
    ChromosomeInfo, StreamHasher,
};
use crate::store::MatchStore;

#[derive(Debug, Serialize, Deserialize)]
pub enum StreamFrame {
    ChromosomeInfo(Vec<Vec<ChromosomeInfo>>),
    Progress(ProgressFrame),
    Complete(CompleteFrame),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProgressFrame {
    pub total_matches: u64,
    pub total_records: u64,
    pub per_genome_records: Vec<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CompleteFrame {
    pub total_matches: u64,
    pub total_records: u64,
    pub per_genome_records: Vec<u64>,
    pub distinct_sequence_count: u64,
}

const MAX_GENOMES: usize = 3;
const MAX_FILES_PER_GENOME: usize = 1000;
const SESSION_TTL: Duration = Duration::from_secs(3600);
const JANITOR_INTERVAL: Duration = Duration::from_secs(300);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);
const PROGRESS_EVERY_N_MATCHES: u64 = 1000;

/// One uploaded file: where it lives on disk and the content hash we computed
/// while streaming the upload. The hash is the cache key for both the xmap
/// qry-id vector and the refineFinal lookup.
#[derive(Debug, Clone)]
pub struct StagedFile {
    pub path: PathBuf,
    pub hash: u64,
}

pub struct Session {
    staging_dir: PathBuf,
    genome_sequence_files: [Vec<StagedFile>; MAX_GENOMES],
    genome_refinefinal: [Option<StagedFile>; MAX_GENOMES],
    pub last_touched: Instant,
    pub store: Arc<MatchStore>,
    pub file_to_genome: Option<Vec<usize>>,
    pub match_complete: bool,
}

impl Session {
    fn new(staging_dir: PathBuf) -> Self {
        Self {
            staging_dir,
            genome_sequence_files: Default::default(),
            genome_refinefinal: Default::default(),
            last_touched: Instant::now(),
            store: Arc::new(MatchStore::new()),
            file_to_genome: None,
            match_complete: false,
        }
    }

    fn touch(&mut self) { self.last_touched = Instant::now(); }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.staging_dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&self.staging_dir) {
                eprintln!(
                    "[session] warning: failed to remove staging dir {:?}: {e:?}",
                    self.staging_dir
                );
            }
        }
    }
}

pub type SessionStore = DashMap<Uuid, Session>;

pub struct AppState {
    pub cache: Arc<XmapCache>,
    pub sessions: Arc<SessionStore>,
}

pub fn spawn_session_janitor(sessions: Arc<SessionStore>, cache: Arc<XmapCache>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(JANITOR_INTERVAL);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let now = Instant::now();
            let expired: Vec<Uuid> = sessions
                .iter()
                .filter(|entry| now.duration_since(entry.value().last_touched) > SESSION_TTL)
                .map(|entry| *entry.key())
                .collect();
            for id in expired {
                eprintln!("[janitor] evicting expired session {id}");
                sessions.remove(&id);
            }

            let (n_xmap, n_rf) = cache.evict_expired();
            if n_xmap > 0 || n_rf > 0 {
                eprintln!(
                    "[janitor] evicted {n_xmap} xmap and {n_rf} refineFinal cache entries; \
                     remaining xmap={}, refineFinal={}",
                    cache.len_xmap(), cache.len_refinefinal()
                );
            }
        }
    });
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateSessionResponse {
    pub session_id: String,
}

pub async fn create_session(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CreateSessionResponse>, StatusCode> {
    let id = Uuid::new_v4();
    let staging_dir = std::env::temp_dir().join(format!("ogm-{id}"));

    if let Err(e) = tokio::fs::create_dir(&staging_dir).await {
        eprintln!("[api] !!! failed to create staging dir {staging_dir:?}: {e:?}");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    state.sessions.insert(id, Session::new(staging_dir));
    eprintln!("[api] [session {id}] created");

    Ok(Json(CreateSessionResponse { session_id: id.to_string() }))
}

pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> StatusCode {
    let uuid = match Uuid::parse_str(&session_id) {
        Ok(u) => u,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    state.sessions.remove(&uuid);
    eprintln!("[api] [session {uuid}] deleted (explicit)");
    StatusCode::OK
}

pub async fn upload_file(
    State(state): State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
    mut multipart: Multipart,
) -> Result<StatusCode, StatusCode> {
    let upload_start = Instant::now();

    let uuid = Uuid::parse_str(&session_id).map_err(|_| {
        eprintln!("[api] !!! upload: malformed session id '{session_id}'");
        StatusCode::BAD_REQUEST
    })?;

    if !state.sessions.contains_key(&uuid) {
        eprintln!("[api] !!! upload: unknown session {uuid}");
        return Err(StatusCode::NOT_FOUND);
    }

    let field = multipart
        .next_field()
        .await
        .map_err(|e| {
            eprintln!("[api] !!! upload: multipart next_field error: {e:?}");
            StatusCode::BAD_REQUEST
        })?
        .ok_or_else(|| {
            eprintln!("[api] !!! upload: no field in multipart body");
            StatusCode::BAD_REQUEST
        })?;

    let field_name = field.name().unwrap_or("").to_string();
    let (genome_index, is_refinefinal) = parse_field_name(&field_name).ok_or_else(|| {
        eprintln!("[api] !!! upload: unknown field name '{field_name}'");
        StatusCode::BAD_REQUEST
    })?;
    if genome_index >= MAX_GENOMES {
        eprintln!("[api] !!! upload: genome_index {genome_index} >= MAX_GENOMES {MAX_GENOMES}");
        return Err(StatusCode::BAD_REQUEST);
    }

    let staging_dir = state.sessions.get(&uuid).ok_or_else(|| {
        eprintln!("[api] !!! upload: session {uuid} disappeared mid-upload");
        StatusCode::NOT_FOUND
    })?.staging_dir.clone();

    let file_uuid = Uuid::new_v4();
    let temp_path = staging_dir.join(format!("{file_uuid}.xmap"));

    let mut file = File::create(&temp_path).await.map_err(|e| {
        eprintln!("[api] !!! upload: File::create failed for {temp_path:?}: {e:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let mut stream = field;
    let mut bytes_written: u64 = 0;
    let mut last_log = Instant::now();
    let mut hasher = StreamHasher::new();
    while let Some(chunk) = stream.chunk().await.map_err(|e| {
        eprintln!(
            "[api] !!! upload: field.chunk() error on '{field_name}' after {bytes_written} bytes: {e:?}"
        );
        StatusCode::BAD_REQUEST
    })? {
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| {
            eprintln!("[api] !!! upload: write_all error on '{field_name}': {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        bytes_written += chunk.len() as u64;
        if last_log.elapsed() > Duration::from_secs(2) {
            eprintln!(
                "[api] [session {uuid}] '{field_name}' progress: {} MiB",
                bytes_written / (1024 * 1024)
            );
            last_log = Instant::now();
        }
    }
    file.flush().await.map_err(|e| {
        eprintln!("[api] !!! upload: flush error on '{field_name}': {e:?}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    drop(file);
    let content_hash = hasher.finish();

    eprintln!(
        "[api] [session {uuid}] '{field_name}' uploaded: {bytes_written} bytes (hash {content_hash:016x}) in {:?}",
        upload_start.elapsed()
    );

    let mut session = state.sessions.get_mut(&uuid).ok_or_else(|| {
        eprintln!(
            "[api] !!! upload: session {uuid} evicted after file write; file orphaned at {temp_path:?}"
        );
        let _ = std::fs::remove_file(&temp_path);
        StatusCode::NOT_FOUND
    })?;

    let staged = StagedFile { path: temp_path, hash: content_hash };
    if is_refinefinal {
        session.genome_refinefinal[genome_index] = Some(staged);
    } else {
        if session.genome_sequence_files[genome_index].len() >= MAX_FILES_PER_GENOME {
            eprintln!(
                "[api] !!! upload: too many sequence files for genome {genome_index} in session {uuid}"
            );
            return Err(StatusCode::BAD_REQUEST);
        }
        session.genome_sequence_files[genome_index].push(staged);
    }
    session.touch();

    Ok(StatusCode::OK)
}

pub async fn stream_matches(
    State(state): State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> Result<Response<Body>, StatusCode> {
    let req_start = Instant::now();
    eprintln!("[api] >>> /api/match request for session '{session_id}'");

    let uuid = Uuid::parse_str(&session_id).map_err(|_| {
        eprintln!("[api] !!! match: malformed session id '{session_id}'");
        StatusCode::BAD_REQUEST
    })?;

    let (genome_sequence_files, mut genome_refinefinal, store_arc) = {
        let mut entry = state.sessions.get_mut(&uuid).ok_or_else(|| {
            eprintln!("[api] !!! match: unknown session {uuid}");
            StatusCode::NOT_FOUND
        })?;

        if entry.match_complete {
            eprintln!("[api] !!! match: session {uuid} already completed");
            return Err(StatusCode::CONFLICT);
        }

        let sequences: Vec<Vec<StagedFile>> =
            std::mem::take(&mut entry.genome_sequence_files).into_iter().collect();
        let refinefinals: Vec<Option<StagedFile>> =
            std::mem::take(&mut entry.genome_refinefinal).into_iter().collect();
        let store = Arc::clone(&entry.store);
        entry.touch();
        (sequences, refinefinals, store)
    };

    let populated_genomes: Vec<usize> = genome_sequence_files
        .iter()
        .enumerate()
        .filter(|(_, files)| !files.is_empty())
        .map(|(i, _)| i)
        .collect();

    eprintln!(
        "[api] populated genomes: {:?} (sequence counts: {:?})",
        populated_genomes,
        genome_sequence_files.iter().map(|v| v.len()).collect::<Vec<_>>()
    );

    if populated_genomes.len() < 2 {
        eprintln!("[api] !!! fewer than 2 populated genomes, rejecting");
        return Err(StatusCode::BAD_REQUEST);
    }

    for &gi in &populated_genomes {
        if genome_refinefinal[gi].is_none() {
            eprintln!("[api] !!! genome {gi} is missing its refineFinal file");
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    // Parse refineFinals — hit the cache first, parse on miss.
    let mut refinefinal_lookups = Vec::new();
    let mut chromosome_info_per_genome: Vec<Vec<ChromosomeInfo>> = Vec::new();

    for &gi in &populated_genomes {
        let rf_staged = genome_refinefinal[gi].take().unwrap();
        let rf_hash = rf_staged.hash;

        let parsed = if let Some(cached) = state.cache.get_refinefinal(rf_hash) {
            eprintln!("[api] [cache] refineFinal hit for genome {gi} (hash {rf_hash:016x})");
            cached
        } else {
            eprintln!("[api] [cache] refineFinal miss for genome {gi} (hash {rf_hash:016x})");
            let cache = Arc::clone(&state.cache);
            tokio::task::spawn_blocking(move || {
                parse_refinefinal_cached(&rf_staged.path, rf_hash, &cache)
            })
                .await
                .map_err(|e| {
                    eprintln!("[api] !!! refineFinal join error for genome {gi}: {e:?}");
                    StatusCode::INTERNAL_SERVER_ERROR
                })?
                .map_err(|e| {
                    eprintln!("[api] !!! refineFinal parse error for genome {gi}: {e:?}");
                    StatusCode::BAD_REQUEST
                })?
        };

        let chr_info: Vec<ChromosomeInfo> = parsed.chr_lengths
            .iter()
            .map(|(&ref_contig_id, &ref_len)| ChromosomeInfo { ref_contig_id, ref_len })
            .collect();

        chromosome_info_per_genome.push(chr_info);
        // Share the cached HashMap with the matcher — no copy on a cache hit.
        refinefinal_lookups.push(Arc::clone(&parsed.lookup));
    }

    // Parse sequence (xmap) files; the hash was computed at upload time.
    let total_sequence_files: usize = populated_genomes
        .iter()
        .map(|&gi| genome_sequence_files[gi].len())
        .sum();
    eprintln!("[api] parsing {total_sequence_files} sequence files");

    let mut all_file_qrys = Vec::new();
    let mut file_to_genome: Vec<usize> = Vec::new();

    for (genome_order_idx, &gi) in populated_genomes.iter().enumerate() {
        for staged in &genome_sequence_files[gi] {
            let hash = staged.hash;

            let qrys = if let Some(cached) = state.cache.get_xmap(hash) {
                eprintln!("[api] [cache] xmap hit (hash {hash:016x})");
                cached
            } else {
                eprintln!("[api] [cache] xmap miss (hash {hash:016x})");
                let cache = Arc::clone(&state.cache);
                let path = staged.path.clone();
                tokio::task::spawn_blocking(move || parse_xmap_disk(&path, hash, &cache))
                    .await
                    .map_err(|e| {
                        eprintln!("[api] !!! parse join error: {e:?}");
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?
                    .map_err(|e| {
                        eprintln!("[api] !!! parse_xmap_disk error: {e:?}");
                        StatusCode::BAD_REQUEST
                    })?
            };

            all_file_qrys.push(qrys);
            file_to_genome.push(genome_order_idx);
        }
    }

    let file_to_genome_for_session = file_to_genome.clone();
    let fileset = Arc::new(XmapFileSet::new(
        all_file_qrys.into_boxed_slice(),
        file_to_genome.into_boxed_slice(),
        refinefinal_lookups.into_boxed_slice(),
    ));

    if let Some(mut entry) = state.sessions.get_mut(&uuid) {
        entry.file_to_genome = Some(file_to_genome_for_session);
        entry.touch();
    }

    let (mut writer, reader) = tokio::io::duplex(1 << 20);
    eprintln!(
        "[api] setup time before spawning stream task: {:?}",
        req_start.elapsed()
    );

    let sessions_for_task = Arc::clone(&state.sessions);
    let store_for_task = Arc::clone(&store_arc);

    tokio::spawn(async move {
        let stream_start = Instant::now();
        let mut writer_alive = true;

        async fn send_frame(
            writer: &mut tokio::io::DuplexStream,
            frame: &StreamFrame,
        ) -> bool {
            let bytes = match bincode::serialize(frame) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("[api] [stream] !!! bincode serialize failed: {e:?}");
                    return false;
                }
            };
            let len = (bytes.len() as u32).to_le_bytes();
            if writer.write_all(&len).await.is_err() { return false; }
            if writer.write_all(&bytes).await.is_err() { return false; }
            writer.flush().await.is_ok()
        }

        // Frame 1: ChromosomeInfo
        let chr_frame = StreamFrame::ChromosomeInfo(chromosome_info_per_genome);
        if !send_frame(&mut writer, &chr_frame).await {
            eprintln!("[api] [stream {uuid}] client disconnected before chromosome info frame");
            writer_alive = false;
        }

        let file_to_genome_slice: Vec<usize> = sessions_for_task
            .get(&uuid)
            .and_then(|s| s.file_to_genome.clone())
            .unwrap_or_default();

        // Bridge crossbeam (sync producer) → tokio mpsc (async consumer).
        let rx = crate::xmap::stream_matches(fileset);
        let (bridge_tx, mut bridge_rx) =
            tokio::sync::mpsc::channel::<crate::xmap::XmapMatch>(256);

        tokio::task::spawn_blocking(move || {
            while let Ok(m) = rx.recv() {
                if bridge_tx.blocking_send(m).is_err() {
                    break;
                }
            }
        });

        let mut progress_ticker = tokio::time::interval(PROGRESS_INTERVAL);
        progress_ticker.tick().await; // skip immediate first tick
        let mut last_progress_matches: u64 = 0;

        loop {
            tokio::select! {
                maybe_match = bridge_rx.recv() => {
                    let Some(m) = maybe_match else { break; };
                    store_for_task.push_match(
                        m.qry_contig_id,
                        &m.records,
                        &file_to_genome_slice,
                    );

                    let snap = store_for_task.snapshot();
                    if writer_alive
                        && snap.total_matches - last_progress_matches >= PROGRESS_EVERY_N_MATCHES
                    {
                        let pf = StreamFrame::Progress(ProgressFrame {
                            total_matches: snap.total_matches,
                            total_records: snap.total_records,
                            per_genome_records: snap.per_genome_records,
                        });
                        if !send_frame(&mut writer, &pf).await {
                            writer_alive = false;
                            eprintln!("[api] [stream {uuid}] client disconnected; continuing ingest");
                        }
                        last_progress_matches = snap.total_matches;
                    }
                }

                _ = progress_ticker.tick() => {
                    if writer_alive {
                        let snap = store_for_task.snapshot();
                        let pf = StreamFrame::Progress(ProgressFrame {
                            total_matches: snap.total_matches,
                            total_records: snap.total_records,
                            per_genome_records: snap.per_genome_records,
                        });
                        if !send_frame(&mut writer, &pf).await {
                            writer_alive = false;
                            eprintln!("[api] [stream {uuid}] client disconnected; continuing ingest");
                        } else {
                            last_progress_matches = snap.total_matches;
                        }
                    }
                }
            }
        }

        let finalize_start = Instant::now();
        store_for_task.finalize(&file_to_genome_slice);
        eprintln!(
            "[api] [stream {uuid}] finalized aggregates in {:?}",
            finalize_start.elapsed()
        );

        if writer_alive {
            let snap = store_for_task.snapshot();
            let distinct = store_for_task.distinct_sequence_count() as u64;
            let cf = StreamFrame::Complete(CompleteFrame {
                total_matches: snap.total_matches,
                total_records: snap.total_records,
                per_genome_records: snap.per_genome_records,
                distinct_sequence_count: distinct,
            });
            if !send_frame(&mut writer, &cf).await {
                eprintln!("[api] [stream {uuid}] client disconnected before Complete frame");
            }
        }

        if let Some(mut entry) = sessions_for_task.get_mut(&uuid) {
            entry.match_complete = true;
            entry.touch();
            let old_dir = std::mem::take(&mut entry.staging_dir);
            drop(entry);
            if !old_dir.as_os_str().is_empty() && old_dir.exists() {
                if let Err(e) = tokio::fs::remove_dir_all(&old_dir).await {
                    eprintln!(
                        "[api] [stream {uuid}] warning: failed to remove staging dir {:?}: {e:?}",
                        old_dir
                    );
                }
            }
        }

        drop(writer);
        let snap = store_for_task.snapshot();
        eprintln!(
            "[api] [stream {uuid}] DONE: {} matches / {} records in {:?} (total request {:?})",
            snap.total_matches, snap.total_records,
            stream_start.elapsed(), req_start.elapsed()
        );
    });

    Ok(Response::builder()
        .header("Content-Type", "application/octet-stream")
        .header("Cache-Control", "no-cache")
        .header("X-Content-Type-Options", "nosniff")
        .body(Body::from_stream(ReaderStream::new(reader)))
        .unwrap())
}

/// Parse the multipart field name. Form is `g{gi}_r` for refineFinal or
/// `g{gi}_s{si}` for sequence files. Returns `(genome_index, is_refinefinal)`.
fn parse_field_name(field_name: &str) -> Option<(usize, bool)> {
    let s = field_name.strip_prefix('g')?;
    let underscore = s.find('_')?;
    let genome_index: usize = s[..underscore].parse().ok()?;
    let suffix = &s[underscore + 1..];

    if suffix == "r" {
        Some((genome_index, true))
    } else if let Some(rest) = suffix.strip_prefix('s') {
        rest.parse::<usize>().ok().map(|_| (genome_index, false))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_field_name_refinefinal() {
        assert_eq!(parse_field_name("g0_r"), Some((0, true)));
        assert_eq!(parse_field_name("g2_r"), Some((2, true)));
    }

    #[test]
    fn parse_field_name_sequence() {
        assert_eq!(parse_field_name("g0_s0"),  Some((0, false)));
        assert_eq!(parse_field_name("g1_s42"), Some((1, false)));
    }

    #[test]
    fn parse_field_name_rejects_garbage() {
        assert_eq!(parse_field_name(""), None);
        assert_eq!(parse_field_name("g"), None);
        assert_eq!(parse_field_name("gX_r"), None);
        assert_eq!(parse_field_name("g0_x"), None);
        assert_eq!(parse_field_name("g0_sX"), None);
        assert_eq!(parse_field_name("0_r"), None);
        assert_eq!(parse_field_name("g0_c0"), None);
    }
}