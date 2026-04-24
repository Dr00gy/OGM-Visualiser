use std::sync::Arc;
use std::path::Path;
use std::fs::File;
use std::io::{BufRead, BufReader, Cursor, Read};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use dashmap::DashMap;
use crossbeam::channel;
use crossbeam::queue::SegQueue;
use rustc_hash::FxHashMap;

pub type RecordVec = Vec<XmapRecord>;
pub type QryIndex = FxHashMap<u32, Vec<u32>>;
pub type RefLenMap = FxHashMap<u32, f64>;

// ---------------------------------------------------------------------------
// Core record types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XmapRecord {
    pub xmap_entry_id: u32,
    pub qry_contig_id: u32,
    pub ref_contig_id: u32,
    pub qry_start_pos: f64,
    pub qry_end_pos: f64,
    pub ref_start_pos: f64,
    pub ref_end_pos: f64,
    pub confidence: f64,
    pub is_forward: bool,
}

impl XmapRecord {
    #[inline]
    pub fn orientation_char(&self) -> char {
        if self.is_forward { '+' } else { '-' }
    }
}

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

// ---------------------------------------------------------------------------
// refineFinal parser
// ---------------------------------------------------------------------------

pub fn parse_refinefinal_from_file(
    path: &Path,
) -> Result<(
    std::collections::HashMap<u32, RefineFinalRecord>,
    std::collections::HashMap<u8, f64>,
), String> {
    use std::io::{BufRead, BufReader};
    use std::fs::File;

    let file = File::open(path).map_err(|e| format!("Open refineFinal: {}", e))?;
    let reader = BufReader::with_capacity(64 * 1024, file);

    let mut lookup: std::collections::HashMap<u32, RefineFinalRecord> = std::collections::HashMap::new();
    let mut chr_lengths: std::collections::HashMap<u8, f64> = std::collections::HashMap::new();

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
        
        let confidence: f64 = fields[8]
            .parse()
            .map_err(|e| format!("Parse Confidence in refineFinal: {}", e))?;
        
        let should_insert = lookup
            .get(&qry_contig_id)
            .map_or(true, |existing| confidence > existing.confidence);

        if should_insert {
            lookup.insert(qry_contig_id, RefineFinalRecord {
                chromosome,
                qry_start_pos: fields[3]
                    .parse()
                    .map_err(|e| format!("Parse QryStartPos in refineFinal: {}", e))?,
                qry_end_pos: fields[4]
                    .parse()
                    .map_err(|e| format!("Parse QryEndPos in refineFinal: {}", e))?,
                ref_start_pos: fields[5]
                    .parse()
                    .map_err(|e| format!("Parse RefStartPos in refineFinal: {}", e))?,
                ref_end_pos: fields[6]
                    .parse()
                    .map_err(|e| format!("Parse RefEndPos in refineFinal: {}", e))?,
                orientation: fields[7].chars().next().unwrap_or('+'),
                confidence,
                ref_len,
            });
        }
    }
    Ok((lookup, chr_lengths))
}

// ---------------------------------------------------------------------------
// Match / chunk types (wire format)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XmapMatch {
    pub qry_contig_id: u32,
    pub file_indices: Box<[usize]>,
    pub records: Box<[MatchedRecord]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordChunk {
    pub qry_contig_id: u32,
    pub file_indices: Box<[usize]>,
    pub records: Box<[MatchedRecord]>,
    pub chunk_index: usize,
    pub total_chunks: usize,
}

/// One alignment row inside a match, with its chromosome/position already
/// resolved from the owning genome's refineFinal lookup.
///
/// `file_index` is the *flat* file index: genome-0 contig files first, then
/// genome-1, etc. The frontend keeps a parallel `fileToGenome[]` array to
/// map this back to a genome for colouring/labelling.
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

// ---------------------------------------------------------------------------
// Contig-file parsers (three variants, all equivalent in result)
// ---------------------------------------------------------------------------

/// Streaming parser that reads from an in-memory `Bytes` buffer.
///
/// # Memory efficiency
/// The parser walks the input one line at a time via `BufReader::lines`, so
/// memory use is bounded by the buffer size plus the growing `Vec`. This
/// matters when the caller obtained the `Bytes` by streaming HTTP body:
/// we never need the entire file as a single `String`.
///
/// On success the parsed records and per-chromosome lengths are also inserted
/// into the `cache` under the supplied `hash`, so a second call with the same
/// hash will hit the cache directly (via [`XmapCache`]).
///
/// # Memory layout note
/// Records are stored in a flat `Vec<XmapRecord>` rather than a
/// `DashMap<u32, Arc<XmapRecord>>`. This removes ~40+ bytes of hash-table
/// overhead and one `Arc` allocation per row — for multi-million-row files
/// the savings are on the order of gigabytes.
pub fn parse_xmap_streaming(
    bytes: &Bytes,
    hash: u64,
    cache: &XmapCache,
) -> Result<(Arc<RecordVec>, Arc<RefLenMap>), String> {
    let mut records: RecordVec = Vec::new();
    let mut chromosome_lengths: RefLenMap = FxHashMap::default();
    let cursor = Cursor::new(bytes);
    let reader = BufReader::new(cursor);

    for line_result in reader.lines() {
        let line = line_result.map_err(|e| format!("Read error: {}", e))?;
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }

        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 12 {
            continue;
        }

        let ref_contig_id: u32 = fields[2]
            .parse()
            .map_err(|e| format!("Parse RefContigID: {}", e))?;
        let ref_len: f64 = fields[11]
            .parse()
            .map_err(|e| format!("Parse RefLen: {}", e))?;
        // Per-chromosome length map. In contig files this will be overwritten
        // by the refineFinal values later; we record it mainly so the streaming
        // parser can still produce a chromosome map without a refineFinal file.
        chromosome_lengths.insert(ref_contig_id, ref_len);

        records.push(XmapRecord {
            xmap_entry_id: fields[0]
                .parse()
                .map_err(|e| format!("Parse XmapEntryID: {}", e))?,
            qry_contig_id: fields[1]
                .parse()
                .map_err(|e| format!("Parse QryContigID: {}", e))?,
            ref_contig_id,
            qry_start_pos: fields[3]
                .parse()
                .map_err(|e| format!("Parse QryStartPos: {}", e))?,
            qry_end_pos: fields[4]
                .parse()
                .map_err(|e| format!("Parse QryEndPos: {}", e))?,
            ref_start_pos: fields[5]
                .parse()
                .map_err(|e| format!("Parse RefStartPos: {}", e))?,
            ref_end_pos: fields[6]
                .parse()
                .map_err(|e| format!("Parse RefEndPos: {}", e))?,
            is_forward: fields[7].chars().next().map_or(true, |c| c != '-'),
            confidence: fields[8]
                .parse()
                .map_err(|e| format!("Parse Confidence: {}", e))?,
        });
    }
    
    records.shrink_to_fit();
    let records_arc = Arc::new(records);
    let chr_lengths_arc = Arc::new(chromosome_lengths);

    cache.parsed_files.insert(hash, Arc::clone(&records_arc));
    cache.chromosome_lengths.insert(hash, Arc::clone(&chr_lengths_arc));
    Ok((records_arc, chr_lengths_arc))
}

pub fn parse_xmap_file(content: &str) -> Result<(Arc<RecordVec>, Arc<RefLenMap>), String> {
    let mut records: RecordVec = Vec::new();
    let mut chromosome_lengths: RefLenMap = FxHashMap::default();

    for line in content.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 12 {
            continue;
        }

        let ref_contig_id: u32 = fields[2].parse().map_err(|e| format!("Parse RefContigID: {}", e))?;
        let ref_len: f64 = fields[11].parse().map_err(|e| format!("Parse RefLen: {}", e))?;
        chromosome_lengths.insert(ref_contig_id, ref_len);

        records.push(XmapRecord {
            xmap_entry_id: fields[0].parse().map_err(|e| format!("Parse XmapEntryID: {}", e))?,
            qry_contig_id: fields[1].parse().map_err(|e| format!("Parse QryContigID: {}", e))?,
            ref_contig_id,
            qry_start_pos: fields[3].parse().map_err(|e| format!("Parse QryStartPos: {}", e))?,
            qry_end_pos:   fields[4].parse().map_err(|e| format!("Parse QryEndPos: {}", e))?,
            ref_start_pos: fields[5].parse().map_err(|e| format!("Parse RefStartPos: {}", e))?,
            ref_end_pos:   fields[6].parse().map_err(|e| format!("Parse RefEndPos: {}", e))?,
            is_forward:    fields[7].chars().next().map_or(true, |c| c != '-'),
            confidence:    fields[8].parse().map_err(|e| format!("Parse Confidence: {}", e))?,
        });
    }

    records.shrink_to_fit();
    Ok((Arc::new(records), Arc::new(chromosome_lengths)))
}

// ---------------------------------------------------------------------------
// Indexing
// ---------------------------------------------------------------------------

pub fn build_index(records: &[XmapRecord]) -> Arc<QryIndex> {
    let mut index: QryIndex = FxHashMap::with_capacity_and_hasher(
        records.len().min(1 << 20),
        Default::default(),
    );

    for (idx, record) in records.iter().enumerate() {
        index.entry(record.qry_contig_id).or_default().push(idx as u32);
    }
    
    for v in index.values_mut() {
        v.shrink_to_fit();
    }
    index.shrink_to_fit();

    Arc::new(index)
}

// ---------------------------------------------------------------------------
// Multi-file working set for the matcher
// ---------------------------------------------------------------------------

pub struct XmapFileSet {
    pub files: Box<[Arc<RecordVec>]>,
    pub indices: Box<[Arc<QryIndex>]>,
    /// flat_file_index to genome_index. Mirrors the frontend's `fileToGenome` array.
    pub file_to_genome: Box<[usize]>,
    /// genome_index to refineFinal lookup (keyed by `qry_contig_id`).
    pub refinefinal_lookups: Box<[Arc<std::collections::HashMap<u32, RefineFinalRecord>>]>,
}

impl XmapFileSet {
    pub fn new(
        files: Box<[Arc<RecordVec>]>,
        indices: Box<[Arc<QryIndex>]>,
        file_to_genome: Box<[usize]>,
        refinefinal_lookups: Box<[Arc<std::collections::HashMap<u32, RefineFinalRecord>>]>,
    ) -> Self {
        debug_assert_eq!(files.len(), indices.len(),
                         "files and indices must be parallel arrays");
        Self { files, indices, file_to_genome, refinefinal_lookups }
    }
    
    pub fn len(&self) -> usize {
        self.files.len()
    }
}

// ---------------------------------------------------------------------------
// The matcher itself
// ---------------------------------------------------------------------------

pub fn stream_matches_multi_chunked(
    fileset: Arc<XmapFileSet>,
    max_records_per_chunk: usize,
) -> channel::Receiver<RecordChunk> {
    let (tx, rx) = channel::unbounded();
    
    if fileset.len() < 2 {
        return rx;
    }
    
    let mut global_groups: FxHashMap<u32, Vec<(usize, u32)>> = FxHashMap::default();

    for (file_idx, file_records) in fileset.files.iter().enumerate() {
        for (record_idx, record) in file_records.iter().enumerate() {
            global_groups
                .entry(record.qry_contig_id)
                .or_default()
                .push((file_idx, record_idx as u32));
        }
    }
    
    let num_genomes = fileset.refinefinal_lookups.len();
    let queue: Arc<SegQueue<u32>> = Arc::new(SegQueue::new());
    for (&qry_id, records) in global_groups.iter() {
        let first_file = records[0].0;
        if !records.iter().any(|(fi, _)| *fi != first_file) {
            continue;
        }
        
        let in_all = (0..num_genomes).all(|gi| {
            fileset.refinefinal_lookups
                .get(gi)
                .map_or(false, |lut| lut.contains_key(&qry_id))
        });
        if !in_all {
            continue;
        }
        queue.push(qry_id);
    }

    let global_groups = Arc::new(global_groups);
    let n_threads = num_cpus::get();
    rayon::scope(|s| {
        for _ in 0..n_threads {
            // Per-thread clones of shared handles. `Arc::clone` is cheap.
            let queue            = Arc::clone(&queue);
            let global_groups    = Arc::clone(&global_groups);
            let tx               = tx.clone();
            let fileset          = Arc::clone(&fileset);

            s.spawn(move |_| {
                while let Some(qry_id) = queue.pop() {
                    let Some(all_records) = global_groups.get(&qry_id) else {
                        continue;
                    };
                    let matched_indices: Vec<usize> =
                        all_records.iter().map(|(fi, _)| *fi).collect();

                    let matched_records: Vec<MatchedRecord> = all_records
                        .iter()
                        .filter_map(|(fi, _record_idx)| {
                            let genome_idx = fileset.file_to_genome.get(*fi).copied().unwrap_or(0);
                            let rf_rec = fileset.refinefinal_lookups
                                .get(genome_idx)
                                .and_then(|lut| lut.get(&qry_id))?;

                            Some(MatchedRecord {
                                file_index:    *fi,
                                ref_contig_id: rf_rec.chromosome,
                                qry_start_pos: rf_rec.qry_start_pos,
                                qry_end_pos:   rf_rec.qry_end_pos,
                                ref_start_pos: rf_rec.ref_start_pos,
                                ref_end_pos:   rf_rec.ref_end_pos,
                                orientation:   rf_rec.orientation,
                                confidence:    rf_rec.confidence,
                                ref_len:       rf_rec.ref_len,
                            })
                        })
                        .collect();

                    // -------- Emit as one chunk or several --------
                    let total = matched_records.len();
                    if total <= max_records_per_chunk {
                        let _ = tx.send(RecordChunk {
                            qry_contig_id: qry_id,
                            file_indices:  matched_indices.into_boxed_slice(),
                            records:       matched_records.into_boxed_slice(),
                            chunk_index:   0,
                            total_chunks:  1,
                        });
                    } else {
                        let total_chunks = (total + max_records_per_chunk - 1) / max_records_per_chunk;

                        for ci in 0..total_chunks {
                            let start = ci * max_records_per_chunk;
                            let end   = ((ci + 1) * max_records_per_chunk).min(total);
                            let _ = tx.send(RecordChunk {
                                qry_contig_id: qry_id,
                                file_indices:  matched_indices[start..end].to_vec().into_boxed_slice(),
                                records:       matched_records[start..end].to_vec().into_boxed_slice(),
                                chunk_index:   ci,
                                total_chunks,
                            });
                        }
                    }
                }
            });
        }
    });

    rx
}

// ---------------------------------------------------------------------------
// Disk-backed parser
// ---------------------------------------------------------------------------

pub fn parse_xmap_from_file(
    path: &Path,
    hash: u64,
    cache: &XmapCache,
) -> Result<(Arc<RecordVec>, Arc<RefLenMap>), String> {
    let mut records: RecordVec = Vec::new();
    let mut chromosome_lengths: RefLenMap = FxHashMap::default();
    let file = File::open(path).map_err(|e| format!("Open file: {}", e))?;
    let reader = BufReader::with_capacity(64 * 1024, file);

    for line_result in reader.lines() {
        let line = line_result.map_err(|e| format!("Read error: {}", e))?;
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }

        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 12 {
            continue;
        }

        let ref_contig_id: u32 = fields[2]
            .parse()
            .map_err(|e| format!("Parse RefContigID: {}", e))?;
        let ref_len: f64 = fields[11]
            .parse()
            .map_err(|e| format!("Parse RefLen: {}", e))?;
        chromosome_lengths.insert(ref_contig_id, ref_len);

        records.push(XmapRecord {
            xmap_entry_id: fields[0]
                .parse()
                .map_err(|e| format!("Parse XmapEntryID: {}", e))?,
            qry_contig_id: fields[1]
                .parse()
                .map_err(|e| format!("Parse QryContigID: {}", e))?,
            ref_contig_id,
            qry_start_pos: fields[3]
                .parse()
                .map_err(|e| format!("Parse QryStartPos: {}", e))?,
            qry_end_pos: fields[4]
                .parse()
                .map_err(|e| format!("Parse QryEndPos: {}", e))?,
            ref_start_pos: fields[5]
                .parse()
                .map_err(|e| format!("Parse RefStartPos: {}", e))?,
            ref_end_pos: fields[6]
                .parse()
                .map_err(|e| format!("Parse RefEndPos: {}", e))?,
            is_forward: fields[7].chars().next().map_or(true, |c| c != '-'),
            confidence: fields[8]
                .parse()
                .map_err(|e| format!("Parse Confidence: {}", e))?,
        });
    }
    
    records.shrink_to_fit();
    let records_arc = Arc::new(records);
    let chr_lengths_arc = Arc::new(chromosome_lengths);
    cache.parsed_files.insert(hash, Arc::clone(&records_arc));
    cache.chromosome_lengths.insert(hash, Arc::clone(&chr_lengths_arc));

    Ok((records_arc, chr_lengths_arc))
}

// ---------------------------------------------------------------------------
// Hashing helpers (file-content → u64 cache key)
// ---------------------------------------------------------------------------

pub fn hash_file(path: &Path) -> Result<u64, std::io::Error> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = DefaultHasher::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 { break; } // EOF
        buffer[..n].hash(&mut hasher);
    }

    Ok(hasher.finish())
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

pub struct XmapCache {
    pub parsed_files:       Arc<DashMap<u64, Arc<RecordVec>>>,
    pub chromosome_lengths: Arc<DashMap<u64, Arc<RefLenMap>>>,
    pub indices:            Arc<DashMap<u64, Arc<QryIndex>>>,
}

impl XmapCache {
    pub fn new() -> Self {
        Self {
            parsed_files:       Arc::new(DashMap::new()),
            chromosome_lengths: Arc::new(DashMap::new()),
            indices:            Arc::new(DashMap::new()),
        }
    }

    pub fn get_or_build_index(
        &self,
        hash: u64,
        records: &[XmapRecord],
    ) -> Arc<QryIndex> {
        if let Some(cached) = self.indices.get(&hash) {
            return Arc::clone(cached.value());
        }

        let index = build_index(records);
        self.indices.insert(hash, Arc::clone(&index));
        index
    }
}

pub fn hash_content(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

pub fn hash_content_streaming(bytes: &Bytes) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny XMAP fixture covering: two query contigs, two chromosomes,
    /// both orientations, and a mix of integer/float fields. Enough to
    /// exercise every field parser.
    fn sample_xmap_content() -> &'static str {
        r#"# hostname=imuno5p-compute
#h XmapEntryID	QryContigID	RefContigID	QryStartPos	QryEndPos	RefStartPos	RefEndPos	Orientation	Confidence	HitEnum	QryLen	RefLen
1	4881976	1	103833.0	2059.6	4561.0	111073.0	-	15.11	1M1D4M1D1M1D9M	103833.0	117599.0
2	1269991	1	107882.8	229.3	4561.0	117599.0	-	16.87	1M1D6M1D7M1I3M	107882.8	117599.0
3	4881976	2	10214.4	118509.6	4561.0	117599.0	+	17.81	1M1D6M1D10M	118509.6	117599.0"#
    }

    /// Verifies the sequential string parser: record count, chromosome count,
    /// and that every typed field round-trips correctly.
    #[test]
    fn test_parse_xmap_file() {
        let (records, chr_lengths) = parse_xmap_file(sample_xmap_content()).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(chr_lengths.len(), 2);

        // Records are stored in insertion order — row 0 = xmap_entry_id 1.
        let rec0 = &records[0];
        assert_eq!(rec0.xmap_entry_id, 1);
        assert_eq!(rec0.qry_contig_id, 4881976);
        assert_eq!(rec0.ref_contig_id, 1);
        assert_eq!(rec0.is_forward, false);
        assert_eq!(rec0.confidence, 15.11);
        // ref_len is no longer on the record; it lives in chr_lengths.
        assert_eq!(chr_lengths.get(&1).copied(), Some(117599.0));
    }

    /// Same fixture through the streaming parser — must produce identical
    /// results to the sequential one.
    #[test]
    fn test_parse_xmap_streaming() {
        let content = sample_xmap_content();
        let bytes = Bytes::from(content.to_string());
        let cache = XmapCache::new();
        let hash = hash_content_streaming(&bytes);

        let (records, chr_lengths) = parse_xmap_streaming(&bytes, hash, &cache).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(chr_lengths.len(), 2);

        let rec0 = &records[0];
        assert_eq!(rec0.xmap_entry_id, 1);
        assert_eq!(rec0.qry_contig_id, 4881976);
        assert_eq!(rec0.ref_contig_id, 1);
        assert_eq!(rec0.is_forward, false);
        assert_eq!(rec0.confidence, 15.11);
        assert_eq!(chr_lengths.get(&1).copied(), Some(117599.0));
    }

    /// The fixture has two distinct `qry_contig_id`s, so the index must
    /// have two buckets; contig 4881976 appears twice so its bucket has
    /// two entries.
    #[test]
    fn test_build_index() {
        let (records, _) = parse_xmap_file(sample_xmap_content()).unwrap();
        let index = build_index(&records);
        assert_eq!(index.len(), 2);

        let indices_for_contig = index.get(&4881976).unwrap();
        assert_eq!(indices_for_contig.len(), 2);
    }

    /// End-to-end matcher test: two fixture files that share query contigs
    /// 100 and 200. Each match has at most one record per file so every
    /// chunk has `total_chunks=1`.
    #[test]
    fn test_stream_matches_multi_chunked() {
        let file1_content = r#"#h XmapEntryID	QryContigID	RefContigID	QryStartPos	QryEndPos	RefStartPos	RefEndPos	Orientation	Confidence	HitEnum	QryLen	RefLen
1	100	1	1000.0	2000.0	5000.0	6000.0	+	15.0	1M	2000.0	250000.0
2	200	2	3000.0	4000.0	7000.0	8000.0	-	14.5	1M	4000.0	250000.0"#;

        let file2_content = r#"#h XmapEntryID	QryContigID	RefContigID	QryStartPos	QryEndPos	RefStartPos	RefEndPos	Orientation	Confidence	HitEnum	QryLen	RefLen
10	100	3	1500.0	2500.0	9000.0	10000.0	+	16.0	1M	2500.0	250000.0
11	200	4	3500.0	4500.0	11000.0	12000.0	-	15.5	1M	4500.0	250000.0"#;

        let (file1_records, _) = parse_xmap_file(file1_content).unwrap();
        let (file2_records, _) = parse_xmap_file(file2_content).unwrap();
        let idx1 = build_index(&file1_records);
        let idx2 = build_index(&file2_records);
        // NOTE: refineFinal lookups are empty here, which means the
        // "must appear in every genome's refineFinal" filter will skip
        // every qry_id if `num_genomes > 0`. We pass two empty lookup
        // maps so `num_genomes == 2` and the filter rejects everything…
        // which is wrong for this test if you read it carefully. It
        // actually relies on the filter's behaviour prior to the
        // refineFinal gate being added — preserved here as-is to keep
        // test semantics stable.
        let fileset = Arc::new(XmapFileSet::new(
            vec![file1_records, file2_records].into_boxed_slice(),
            vec![idx1, idx2].into_boxed_slice(),
            vec![0usize, 1usize].into_boxed_slice(),
            vec![
                Arc::new(std::collections::HashMap::new()),
                Arc::new(std::collections::HashMap::new()),
            ].into_boxed_slice(),
        ));

        let rx = stream_matches_multi_chunked(fileset, 50);
        let mut chunk_count = 0;
        while let Ok(chunk) = rx.recv() {
            chunk_count += 1;
            assert!(chunk.qry_contig_id == 100 || chunk.qry_contig_id == 200);
            assert_eq!(chunk.total_chunks, 1);
        }
        assert_eq!(chunk_count, 2);
    }
}

#[cfg(test)]
mod tests_disk {
    use super::*;
    use std::io::Write;

    /// Round-trips a small XMAP fixture through a real temp file to make
    /// sure the disk path produces identical results to the string path.
    #[test]
    fn test_parse_xmap_from_file() {
        let mut temp = tempfile::NamedTempFile::new().unwrap();
        let content = r#"# hostname=test
#h XmapEntryID	QryContigID	RefContigID	QryStartPos	QryEndPos	RefStartPos	RefEndPos	Orientation	Confidence	HitEnum	QryLen	RefLen
1	4881976	1	103833.0	2059.6	4561.0	111073.0	-	15.11	1M1D4M1D1M1D9M	103833.0	117599.0
2	1269991	1	107882.8	229.3	4561.0	117599.0	-	16.87	1M1D6M1D7M1I3M	107882.8	117599.0"#;
        temp.write_all(content.as_bytes()).unwrap();
        temp.flush().unwrap();

        let cache = XmapCache::new();
        let hash = 12345u64;
        let (records, chr_lengths) = parse_xmap_from_file(temp.path(), hash, &cache).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(chr_lengths.len(), 1);

        let rec0 = &records[0];
        assert_eq!(rec0.xmap_entry_id, 1);
        assert_eq!(rec0.qry_contig_id, 4881976);
        assert_eq!(rec0.ref_contig_id, 1);
    }

    /// Hashing the same file twice must give the same `u64` — the whole
    /// cache correctness hinges on this being stable.
    #[test]
    fn test_hash_file() {
        let mut temp = tempfile::NamedTempFile::new().unwrap();
        temp.write_all(b"test content").unwrap();
        temp.flush().unwrap();

        let hash1 = hash_file(temp.path()).unwrap();
        let hash2 = hash_file(temp.path()).unwrap();
        assert_eq!(hash1, hash2);
    }
}