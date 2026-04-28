use std::sync::Arc;
use std::path::Path;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
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
    cache.parsed_files.insert(hash, Arc::clone(&arc));
    Ok(arc)
}

pub fn hash_file(path: &Path) -> Result<u64, std::io::Error> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = DefaultHasher::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 { break; }
        buffer[..n].hash(&mut hasher);
    }
    Ok(hasher.finish())
}

pub struct XmapCache {
    pub parsed_files: Arc<DashMap<u64, Arc<XmapQryIds>>>,
}

impl XmapCache {
    pub fn new() -> Self {
        Self { parsed_files: Arc::new(DashMap::new()) }
    }
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
    fn hash_file_is_stable() {
        let temp = write_temp("test content");
        assert_eq!(hash_file(temp.path()).unwrap(), hash_file(temp.path()).unwrap());
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