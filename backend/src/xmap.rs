use std::sync::Arc;
use std::path::Path;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::Hasher;
use std::time::Instant;
use serde::{Deserialize, Serialize};
use dashmap::DashMap;
use crossbeam::channel;
use crossbeam::queue::SegQueue;
use rustc_hash::FxHashMap;

/// One xmap row reduced to what the matcher needs: the query-sequence id.
/// Other columns (positions, confidence, orientation) are read from the
/// refineFinal file at match time and are not retained from the xmap.
pub type XmapQryIds = Vec<u32>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChromosomeInfo {
    pub ref_contig_id: u8,
    pub ref_len: f64,
}

#[derive(Debug, Clone)]
pub struct RefineFinalRecord {
    pub chromosome: u8,
    pub qry_start_pos: f64,
    pub qry_end_pos: f64,
    pub ref_start_pos: f64,
    pub ref_end_pos: f64,
    pub orientation: char,
    pub confidence: f64,
    pub ref_len: f64,
}

/// One alignment row, with chromosome/position resolved from the genome's
/// refineFinal lookup. `file_index` is the flat index across all genomes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedRecord {
    pub file_index: usize,
    pub ref_contig_id: u8,
    pub qry_start_pos: f64,
    pub qry_end_pos: f64,
    pub ref_start_pos: f64,
    pub ref_end_pos: f64,
    pub orientation: char,
    pub confidence: f64,
    pub ref_len: f64,
}

/// One resolved match for a single qry sequence id, ready to push into the store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XmapMatch {
    pub qry_contig_id: u32,
    pub file_indices: Box<[usize]>,
    pub records: Box<[MatchedRecord]>,
}

pub fn parse_refinefinal(
    path: &Path,
) -> Result<(HashMap<u32, Vec<RefineFinalRecord>>, HashMap<u8, f64>), String> {
    let file = File::open(path).map_err(|e| format!("Open refineFinal: {}", e))?;
    let reader = BufReader::with_capacity(64 * 1024, file);

    let mut lookup: HashMap<u32, Vec<RefineFinalRecord>> = HashMap::new();
    let mut chr_lengths: HashMap<u8, f64> = HashMap::new();

    for line_result in reader.lines() {
        let line = line_result.map_err(|e| format!("Read error: {}", e))?;
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }

        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 12 {
            continue;
        }

        let qry_contig_id: u32 = fields[1]
            .parse()
            .map_err(|e| format!("Parse QryContigID in refineFinal: {}", e))?;
        let chromosome: u8 = fields[2]
            .parse()
            .map_err(|e| format!("Parse RefContigID (chromosome) in refineFinal: {}", e))?;
        let ref_len: f64 = fields[11]
            .parse()
            .map_err(|e| format!("Parse RefLen in refineFinal: {}", e))?;

        chr_lengths.insert(chromosome, ref_len);

        lookup.entry(qry_contig_id).or_default().push(RefineFinalRecord {
            chromosome,
            qry_start_pos: fields[3].parse().map_err(|e| format!("Parse QryStartPos: {}", e))?,
            qry_end_pos:   fields[4].parse().map_err(|e| format!("Parse QryEndPos: {}", e))?,
            ref_start_pos: fields[5].parse().map_err(|e| format!("Parse RefStartPos: {}", e))?,
            ref_end_pos:   fields[6].parse().map_err(|e| format!("Parse RefEndPos: {}", e))?,
            orientation:   fields[7].chars().next().unwrap_or('+'),
            confidence:    fields[8].parse().map_err(|e| format!("Parse Confidence: {}", e))?,
            ref_len,
        });
    }
    Ok((lookup, chr_lengths))
}

/// Parse an xmap file and extract just the qry_contig_id column. Result is
/// cached in `cache` keyed by `hash`.
pub fn parse_xmap_disk(
    path: &Path,
    hash: u64,
    cache: &XmapCache,
) -> Result<Arc<XmapQryIds>, String> {
    let file = File::open(path).map_err(|e| format!("Open file: {}", e))?;
    let reader = BufReader::with_capacity(64 * 1024, file);

    let mut qry_ids: XmapQryIds = Vec::new();

    for line_result in reader.lines() {
        let line = line_result.map_err(|e| format!("Read error: {}", e))?;
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }

        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 12 {
            continue;
        }

        let qry: u32 = fields[1]
            .parse()
            .map_err(|e| format!("Parse QryContigID: {}", e))?;
        qry_ids.push(qry);
    }

    qry_ids.shrink_to_fit();
    let arc = Arc::new(qry_ids);
    cache.insert_xmap(hash, Arc::clone(&arc));
    Ok(arc)
}

/// Parse a refineFinal and store the result in the cache, returning the same
/// `Arc` we'd hand out on a hit.
pub fn parse_refinefinal_cached(
    path: &Path,
    hash: u64,
    cache: &XmapCache,
) -> Result<Arc<RefineFinalParsed>, String> {
    let (lookup, chr_lengths) = parse_refinefinal(path)?;
    let arc = Arc::new(RefineFinalParsed {
        lookup: Arc::new(lookup),
        chr_lengths,
    });
    cache.insert_refinefinal(hash, Arc::clone(&arc));
    Ok(arc)
}

/// Hash a file's contents. Uses `Hasher::write` directly (rather than
/// `[u8]::hash`, which prefixes the slice length) so the result is independent
/// of how many bytes are read per `read()` call. This must match what
/// `StreamHasher` does on the upload path so cache lookups hit.
pub fn hash_file(path: &Path) -> Result<u64, std::io::Error> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = DefaultHasher::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 { break; }
        hasher.write(&buffer[..n]);
    }
    Ok(hasher.finish())
}

/// Streaming wrapper around `DefaultHasher` so we can compute the same hash
/// while writing the upload to disk. Uses `Hasher::write` so chunk boundaries
/// don't influence the result — `update(b"hello")` and
/// `update(b"hel"); update(b"lo")` produce the same hash.
#[derive(Default)]
pub struct StreamHasher {
    inner: DefaultHasher,
}

impl StreamHasher {
    pub fn new() -> Self { Self { inner: DefaultHasher::new() } }
    pub fn update(&mut self, bytes: &[u8]) { self.inner.write(bytes); }
    pub fn finish(self) -> u64 { self.inner.finish() }
}

/// Cached parsed refineFinal: the qry-keyed lookup the matcher uses, plus the
/// per-chromosome reference lengths we surface in the first stream frame.
pub struct RefineFinalParsed {
    /// Wrapped in `Arc` so we can hand a clone to the matcher's `XmapFileSet`
    /// without copying the whole HashMap on a cache hit.
    pub lookup: Arc<HashMap<u32, Vec<RefineFinalRecord>>>,
    pub chr_lengths: HashMap<u8, f64>,
}

/// A cache entry that records when it was last touched. Touched means inserted
/// or read; the janitor evicts entries whose `last_touched` exceeds a TTL.
struct CacheEntry<T> {
    value: Arc<T>,
    last_touched: std::sync::Mutex<Instant>,
}

impl<T> CacheEntry<T> {
    fn new(value: Arc<T>) -> Self {
        Self {
            value,
            last_touched: std::sync::Mutex::new(Instant::now()),
        }
    }
    fn touch(&self) {
        if let Ok(mut g) = self.last_touched.lock() { *g = Instant::now(); }
    }
    fn age(&self) -> std::time::Duration {
        self.last_touched
            .lock()
            .map(|g| g.elapsed())
            .unwrap_or_default()
    }
}

/// Process-wide cache for parsed xmap and refineFinal files. Entries live for
/// `XmapCache::TTL` past their last access; the janitor (in `api.rs`) sweeps
/// expired entries periodically. Sharing across sessions is intentional —
/// content-hash keys mean two users uploading the same file get the same
/// parsed `Arc`.
pub struct XmapCache {
    parsed_files: DashMap<u64, CacheEntry<XmapQryIds>>,
    parsed_refinefinals: DashMap<u64, CacheEntry<RefineFinalParsed>>,
}

impl XmapCache {
    pub const TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

    pub fn new() -> Self {
        Self {
            parsed_files: DashMap::new(),
            parsed_refinefinals: DashMap::new(),
        }
    }

    pub fn get_xmap(&self, hash: u64) -> Option<Arc<XmapQryIds>> {
        let entry = self.parsed_files.get(&hash)?;
        entry.touch();
        Some(Arc::clone(&entry.value))
    }

    pub fn insert_xmap(&self, hash: u64, value: Arc<XmapQryIds>) {
        self.parsed_files.insert(hash, CacheEntry::new(value));
    }

    pub fn get_refinefinal(&self, hash: u64) -> Option<Arc<RefineFinalParsed>> {
        let entry = self.parsed_refinefinals.get(&hash)?;
        entry.touch();
        Some(Arc::clone(&entry.value))
    }

    pub fn insert_refinefinal(&self, hash: u64, value: Arc<RefineFinalParsed>) {
        self.parsed_refinefinals.insert(hash, CacheEntry::new(value));
    }

    /// Remove entries whose `last_touched` exceeds `Self::TTL`. Returns
    /// `(xmap_evicted, refinefinal_evicted)` for logging.
    pub fn evict_expired(&self) -> (usize, usize) {
        let xmap_keys: Vec<u64> = self
            .parsed_files
            .iter()
            .filter(|e| e.value().age() > Self::TTL)
            .map(|e| *e.key())
            .collect();
        for k in &xmap_keys { self.parsed_files.remove(k); }

        let rf_keys: Vec<u64> = self
            .parsed_refinefinals
            .iter()
            .filter(|e| e.value().age() > Self::TTL)
            .map(|e| *e.key())
            .collect();
        for k in &rf_keys { self.parsed_refinefinals.remove(k); }

        (xmap_keys.len(), rf_keys.len())
    }

    pub fn len_xmap(&self) -> usize { self.parsed_files.len() }
    pub fn len_refinefinal(&self) -> usize { self.parsed_refinefinals.len() }
}

pub struct XmapFileSet {
    pub files: Box<[Arc<XmapQryIds>]>,
    /// flat file index → genome index.
    pub file_to_genome: Box<[usize]>,
    /// genome index → refineFinal lookup (keyed by qry_contig_id).
    pub refinefinal: Box<[Arc<HashMap<u32, Vec<RefineFinalRecord>>>]>,
}

impl XmapFileSet {
    pub fn new(
        files: Box<[Arc<XmapQryIds>]>,
        file_to_genome: Box<[usize]>,
        refinefinal: Box<[Arc<HashMap<u32, Vec<RefineFinalRecord>>>]>,
    ) -> Self {
        Self { files, file_to_genome, refinefinal }
    }

    pub fn len(&self) -> usize { self.files.len() }
}

/// Group qry rows across files, keep only those that span >1 file and exist
/// in every genome's refineFinal, then resolve each via refineFinal and emit
/// one `XmapMatch` per qry on the returned receiver.
pub fn stream_matches(fileset: Arc<XmapFileSet>) -> channel::Receiver<XmapMatch> {
    let (tx, rx) = channel::unbounded();
    if fileset.len() < 2 {
        return rx;
    }

    let mut groups: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    for (file_idx, file_qrys) in fileset.files.iter().enumerate() {
        for &qry in file_qrys.iter() {
            groups.entry(qry).or_default().push(file_idx);
        }
    }

    let n_genomes = fileset.refinefinal.len();
    let queue: Arc<SegQueue<u32>> = Arc::new(SegQueue::new());
    for (&qry_id, file_indices) in groups.iter() {
        let first_file = file_indices[0];
        if !file_indices.iter().any(|fi| *fi != first_file) {
            continue;
        }
        let in_all = (0..n_genomes).all(|gi| {
            fileset.refinefinal.get(gi).map_or(false, |lut| lut.contains_key(&qry_id))
        });
        if !in_all { continue; }
        queue.push(qry_id);
    }

    let groups = Arc::new(groups);
    let n_threads = num_cpus::get();
    rayon::scope(|s| {
        for _ in 0..n_threads {
            let queue   = Arc::clone(&queue);
            let groups  = Arc::clone(&groups);
            let tx      = tx.clone();
            let fileset = Arc::clone(&fileset);

            s.spawn(move |_| {
                while let Some(qry_id) = queue.pop() {
                    let Some(file_indices) = groups.get(&qry_id) else {
                        continue;
                    };

                    let records: Vec<MatchedRecord> = file_indices
                        .iter()
                        .flat_map(|fi| {
                            let gi = fileset.file_to_genome.get(*fi).copied().unwrap_or(0);
                            let rf_recs = &fileset.refinefinal[gi][&qry_id];
                            rf_recs.iter().map(move |rf| MatchedRecord {
                                file_index:    *fi,
                                ref_contig_id: rf.chromosome,
                                qry_start_pos: rf.qry_start_pos,
                                qry_end_pos:   rf.qry_end_pos,
                                ref_start_pos: rf.ref_start_pos,
                                ref_end_pos:   rf.ref_end_pos,
                                orientation:   rf.orientation,
                                confidence:    rf.confidence,
                                ref_len:       rf.ref_len,
                            })
                        })
                        .collect();

                    let _ = tx.send(XmapMatch {
                        qry_contig_id: qry_id,
                        file_indices: file_indices.clone().into_boxed_slice(),
                        records: records.into_boxed_slice(),
                    });
                }
            });
        }
    });

    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(content: &str) -> tempfile::NamedTempFile {
        let mut t = tempfile::NamedTempFile::new().unwrap();
        t.write_all(content.as_bytes()).unwrap();
        t.flush().unwrap();
        t
    }

    fn sample_xmap() -> &'static str {
        "# hostname=test\n\
#h XmapEntryID\tQryContigID\tRefContigID\tQryStartPos\tQryEndPos\tRefStartPos\tRefEndPos\tOrientation\tConfidence\tHitEnum\tQryLen\tRefLen
1\t4881976\t1\t103833.0\t2059.6\t4561.0\t111073.0\t-\t15.11\t1M\t103833.0\t117599.0
2\t1269991\t1\t107882.8\t229.3\t4561.0\t117599.0\t-\t16.87\t1M\t107882.8\t117599.0
3\t4881976\t2\t10214.4\t118509.6\t4561.0\t117599.0\t+\t17.81\t1M\t118509.6\t117599.0"
    }

    #[test]
    fn parses_xmap_qry_ids() {
        let temp = write_temp(sample_xmap());
        let cache = XmapCache::new();
        let qrys = parse_xmap_disk(temp.path(), 1, &cache).unwrap();
        assert_eq!(&*qrys, &[4881976u32, 1269991, 4881976]);
    }

    #[test]
    fn xmap_cache_hits_after_parse() {
        let temp = write_temp(sample_xmap());
        let cache = XmapCache::new();
        let h = hash_file(temp.path()).unwrap();
        assert!(cache.get_xmap(h).is_none());

        let parsed = parse_xmap_disk(temp.path(), h, &cache).unwrap();
        let cached = cache.get_xmap(h).expect("expected cache hit");
        assert!(Arc::ptr_eq(&parsed, &cached));
    }

    #[test]
    fn cache_evict_expired_drops_entries_past_ttl() {
        let cache = XmapCache::new();
        cache.insert_xmap(7, Arc::new(vec![1u32, 2, 3]));
        assert_eq!(cache.len_xmap(), 1);

        // Force the entry's last_touched into the past.
        if let Some(e) = cache.parsed_files.get(&7) {
            *e.value().last_touched.lock().unwrap() =
                Instant::now() - (XmapCache::TTL + std::time::Duration::from_secs(1));
        }
        let (n_xmap, n_rf) = cache.evict_expired();
        assert_eq!((n_xmap, n_rf), (1, 0));
        assert_eq!(cache.len_xmap(), 0);
    }

    #[test]
    fn hash_file_is_stable() {
        let temp = write_temp("test content");
        assert_eq!(hash_file(temp.path()).unwrap(), hash_file(temp.path()).unwrap());
    }

    #[test]
    fn stream_hasher_matches_hash_file_irrespective_of_chunking() {
        let payload = b"the quick brown fox jumps over the lazy dog 1234567890".repeat(1000);
        let temp = write_temp(std::str::from_utf8(&payload).unwrap());
        let disk = hash_file(temp.path()).unwrap();

        // Hash the same bytes in arbitrarily-sized chunks.
        let mut h = StreamHasher::new();
        let mut i = 0;
        for chunk_size in [1, 17, 4096, 7, 8191, 1].iter().cycle() {
            if i >= payload.len() { break; }
            let end = (i + *chunk_size).min(payload.len());
            h.update(&payload[i..end]);
            i = end;
        }
        assert_eq!(disk, h.finish());
    }

    #[test]
    fn streams_matches_across_files() {
        let f1 = write_temp("#h XmapEntryID\tQryContigID\tRefContigID\tQryStartPos\tQryEndPos\tRefStartPos\tRefEndPos\tOrientation\tConfidence\tHitEnum\tQryLen\tRefLen
1\t100\t1\t1000.0\t2000.0\t5000.0\t6000.0\t+\t15.0\t1M\t2000.0\t250000.0
2\t200\t2\t3000.0\t4000.0\t7000.0\t8000.0\t-\t14.5\t1M\t4000.0\t250000.0");
        let f2 = write_temp("#h XmapEntryID\tQryContigID\tRefContigID\tQryStartPos\tQryEndPos\tRefStartPos\tRefEndPos\tOrientation\tConfidence\tHitEnum\tQryLen\tRefLen
10\t100\t3\t1500.0\t2500.0\t9000.0\t10000.0\t+\t16.0\t1M\t2500.0\t250000.0
11\t200\t4\t3500.0\t4500.0\t11000.0\t12000.0\t-\t15.5\t1M\t4500.0\t250000.0");

        let cache = XmapCache::new();
        let q1 = parse_xmap_disk(f1.path(), 1, &cache).unwrap();
        let q2 = parse_xmap_disk(f2.path(), 2, &cache).unwrap();

        let mut rf: HashMap<u32, Vec<RefineFinalRecord>> = HashMap::new();
        for &qry in &[100u32, 200] {
            rf.insert(qry, vec![RefineFinalRecord {
                chromosome: 1, qry_start_pos: 0.0, qry_end_pos: 0.0,
                ref_start_pos: 0.0, ref_end_pos: 0.0, orientation: '+',
                confidence: 15.0, ref_len: 250000.0,
            }]);
        }
        let rf_arc = Arc::new(rf);

        let fileset = Arc::new(XmapFileSet::new(
            vec![q1, q2].into_boxed_slice(),
            vec![0usize, 1usize].into_boxed_slice(),
            vec![Arc::clone(&rf_arc), Arc::clone(&rf_arc)].into_boxed_slice(),
        ));

        let rx = stream_matches(fileset);
        let mut count = 0;
        while let Ok(m) = rx.recv() {
            count += 1;
            assert!(m.qry_contig_id == 100 || m.qry_contig_id == 200);
        }
        assert_eq!(count, 2);
    }
}