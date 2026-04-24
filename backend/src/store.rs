use crate::xmap::MatchedRecord;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Default)]
pub struct MatchStore {
    inner: RwLock<MatchStoreInner>,
}

#[derive(Default)]
pub struct MatchStoreInner {
    // ------------- Columns (one entry per resolved record) -------------
    /// Flat file index the record came from. Used to resolve genome.
    pub file_index:    Vec<u32>,
    /// Chromosome (1..24 for human).
    pub ref_contig_id: Vec<u8>,
    /// Query start position (bp).
    pub qry_start_pos: Vec<f64>,
    /// Query end position (bp).
    pub qry_end_pos:   Vec<f64>,
    /// Reference start position (bp).
    pub ref_start_pos: Vec<f64>,
    /// Reference end position (bp).
    pub ref_end_pos:   Vec<f64>,
    /// See [`encode_orientation`] / [`decode_orientation`].
    pub orientation:   Vec<u8>,
    /// Alignment confidence.
    pub confidence:    Vec<f64>,
    /// Length of the reference chromosome (bp).
    pub ref_len:       Vec<f64>,
    pub qry_contig_id: Vec<u32>,

    // ------------- Per-match metadata (one entry per match) -------------
    pub match_contig_ids: Vec<u32>,
    pub match_file_indices: Vec<Vec<u32>>,
    pub match_record_counts: Vec<u32>,

    // ------------- Indexes (built during ingestion) -------------
    pub by_contig: HashMap<u32, Vec<u32>>,
    pub total_records: u64,
    pub total_matches: u64,
    pub per_genome_records: Vec<u64>,
    pub finalized: bool,
    pub max_confidence: f64,
    pub available_contig_ids: Vec<u32>,
    pub contig_aggregates: Vec<ContigAggregate>,
    pub contig_aggregate_index: HashMap<u32, usize>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContigAggregate {
    /// The query contig id (what the frontend paginates over).
    pub qry_contig_id: u32,
    /// Total record occurrences across all files/genomes.
    pub total_occurrences: u32,
    /// `per_genome[gi]` = count of records for this contig in genome `gi`.
    /// Short vec (typically 2-3 entries); dense, zero-filled.
    pub per_genome: Vec<u32>,
    /// Per-(genome, chromosome) occurrence counts.
    /// Key format matches the frontend's "gi-chr" composite for
    /// substring search on `chromosomeSearchType`.
    pub per_chromosome: Vec<ChromosomeCount>,
    /// Max confidence seen across all records of this contig.
    pub max_confidence: f64,
}

/// One entry inside `ContigAggregate.per_chromosome`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChromosomeCount {
    /// The genome index (0..N_genomes).
    pub genome_index: u32,
    /// The chromosome number (1..24 for human).
    pub chromosome: u8,
    /// Records of this contig landing in this (genome, chromosome) pair.
    pub count: u32,
}

impl MatchStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push_match(
        &self,
        qry_contig_id: u32,
        file_indices: &[u32],
        records: &[MatchedRecord],
        file_to_genome: &[usize],
    ) {

        if records.is_empty() {
            return;
        }

        let mut inner = self.inner.write().expect("MatchStore poisoned");
        let start_row = inner.file_index.len() as u32;
        
        let n = records.len();
        inner.file_index.reserve(n);
        inner.ref_contig_id.reserve(n);
        inner.qry_start_pos.reserve(n);
        inner.qry_end_pos.reserve(n);
        inner.ref_start_pos.reserve(n);
        inner.ref_end_pos.reserve(n);
        inner.orientation.reserve(n);
        inner.confidence.reserve(n);
        inner.ref_len.reserve(n);
        inner.qry_contig_id.reserve(n);

        for r in records.iter() {
            inner.file_index.push(r.file_index as u32);
            inner.ref_contig_id.push(r.ref_contig_id);
            inner.qry_start_pos.push(r.qry_start_pos);
            inner.qry_end_pos.push(r.qry_end_pos);
            inner.ref_start_pos.push(r.ref_start_pos);
            inner.ref_end_pos.push(r.ref_end_pos);
            inner.orientation.push(encode_orientation(r.orientation));
            inner.confidence.push(r.confidence);
            inner.ref_len.push(r.ref_len);
            inner.qry_contig_id.push(qry_contig_id);
            
            let gi = file_to_genome.get(r.file_index).copied().unwrap_or(0);
            if inner.per_genome_records.len() <= gi {
                inner.per_genome_records.resize(gi + 1, 0);
            }
            inner.per_genome_records[gi] += 1;
        }
        
        let end_row = start_row + n as u32;
        let bucket = inner.by_contig.entry(qry_contig_id).or_insert_with(Vec::new);
        bucket.extend(start_row..end_row);
        
        inner.match_contig_ids.push(qry_contig_id);
        inner.match_file_indices.push(file_indices.to_vec());
        inner.match_record_counts.push(n as u32);

        inner.total_records += n as u64;
        inner.total_matches += 1;
    }
    
    pub fn progress_snapshot(&self) -> ProgressSnapshot {
        let inner = self.inner.read().expect("MatchStore poisoned");
        ProgressSnapshot {
            total_matches: inner.total_matches,
            total_records: inner.total_records,
            per_genome_records: inner.per_genome_records.clone(),
        }
    }
    
    pub fn distinct_contig_count(&self) -> usize {
        self.inner.read().expect("MatchStore poisoned").by_contig.len()
    }

    pub fn finalize(&self, file_to_genome: &[usize]) {
        let mut inner = self.inner.write().expect("MatchStore poisoned");
        if inner.finalized {
            return;
        }
        
        let max_confidence = inner
            .confidence
            .iter()
            .copied()
            .fold(0.0_f64, f64::max);
        inner.max_confidence = max_confidence;

        let mut ids: Vec<u32> = inner.by_contig.keys().copied().collect();
        ids.sort_unstable();
        inner.available_contig_ids = ids;
        let mut aggregates: Vec<ContigAggregate> =
            Vec::with_capacity(inner.by_contig.len());
        let n_genomes = file_to_genome.iter().copied().max().map(|m| m + 1).unwrap_or(0);
        let contig_ids: Vec<u32> = inner.by_contig.keys().copied().collect();

        for &qry_id in &contig_ids {
            let rows: Vec<u32> = inner.by_contig[&qry_id].clone();

            let mut per_genome = vec![0u32; n_genomes];
            let mut per_chr: HashMap<u32, u32> = HashMap::new();
            let mut max_conf = 0.0_f64;

            for &row in &rows {
                let ri = row as usize;
                let file_idx = inner.file_index[ri] as usize;
                let gi = file_to_genome.get(file_idx).copied().unwrap_or(0);
                let chr = inner.ref_contig_id[ri];
                let conf = inner.confidence[ri];

                if gi < per_genome.len() {
                    per_genome[gi] += 1;
                }
                
                let key = ((gi as u32) << 8) | (chr as u32);
                *per_chr.entry(key).or_insert(0) += 1;

                if conf > max_conf { max_conf = conf; }
            }
            
            let mut chr_counts: Vec<ChromosomeCount> = per_chr
                .into_iter()
                .map(|(key, count)| ChromosomeCount {
                    genome_index: key >> 8,
                    chromosome:   (key & 0xFF) as u8,
                    count,
                })
                .collect();
            chr_counts.sort_unstable_by(|a, b| {
                a.genome_index.cmp(&b.genome_index)
                    .then_with(|| a.chromosome.cmp(&b.chromosome))
            });

            aggregates.push(ContigAggregate {
                qry_contig_id: qry_id,
                total_occurrences: rows.len() as u32,
                per_genome,
                per_chromosome: chr_counts,
                max_confidence: max_conf,
            });
        }
        
        aggregates.sort_by(|a, b| {
            b.total_occurrences.cmp(&a.total_occurrences)
                .then_with(|| a.qry_contig_id.cmp(&b.qry_contig_id))
        });
        
        let mut index: HashMap<u32, usize> = HashMap::with_capacity(aggregates.len());
        for (pos, agg) in aggregates.iter().enumerate() {
            index.insert(agg.qry_contig_id, pos);
        }

        inner.contig_aggregates = aggregates;
        inner.contig_aggregate_index = index;
        inner.finalized = true;
    }
    
    pub fn contig_at(&self, position: usize) -> Option<ContigAggregate> {
        let inner = self.inner.read().expect("MatchStore poisoned");
        inner.contig_aggregates.get(position).cloned()
    }
    
    pub fn scan_contigs<F, T>(&self, mut f: F) -> Vec<T>
    where
        F: FnMut(&ContigAggregate) -> Option<T>,
    {
        let inner = self.inner.read().expect("MatchStore poisoned");
        let mut out = Vec::new();
        for agg in &inner.contig_aggregates {
            if let Some(item) = f(agg) {
                out.push(item);
            }
        }
        out
    }
    
    pub fn scan_and_paginate<F>(
        &self,
        start: usize,
        per_page: usize,
        mut predicate: F,
    ) -> (u64, Vec<ContigAggregate>)
    where
        F: FnMut(&ContigAggregate) -> bool,
    {
        let inner = self.inner.read().expect("MatchStore poisoned");
        let mut total: u64 = 0;
        let end_exclusive = start.saturating_add(per_page);
        let mut page_items: Vec<ContigAggregate> = Vec::with_capacity(per_page);

        for agg in &inner.contig_aggregates {
            if !predicate(agg) { continue; }
            let idx = total as usize;
            if idx >= start && idx < end_exclusive {
                page_items.push(agg.clone());
            }
            total += 1;
        }

        (total, page_items)
    }
    
    pub fn max_confidence(&self) -> f64 {
        let inner = self.inner.read().expect("MatchStore poisoned");
        if inner.max_confidence > 0.0 { inner.max_confidence } else { 1.0 }
    }
    
    pub fn available_contig_ids(&self) -> Vec<u32> {
        self.inner.read().expect("MatchStore poisoned").available_contig_ids.clone()
    }
    
    pub fn rows_for_contig(&self, qry_contig_id: u32) -> Option<Vec<u32>> {
        let inner = self.inner.read().expect("MatchStore poisoned");
        inner.by_contig.get(&qry_contig_id).cloned()
    }
    
    pub fn with_read<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&MatchStoreInner) -> T,
    {
        let inner = self.inner.read().expect("MatchStore poisoned");
        f(&*inner)
    }
}

#[derive(Debug, Clone)]
pub struct ProgressSnapshot {
    pub total_matches:      u64,
    pub total_records:      u64,
    pub per_genome_records: Vec<u64>,
}

// ---------------------------------------------------------------------------
// Orientation packing
// ---------------------------------------------------------------------------

#[inline]
pub fn encode_orientation(c: char) -> u8 {
    match c {
        '-' => 1,
        _ => 0,
    }
}
#[inline]
pub fn decode_orientation(v: u8) -> char {
    if v == 1 { '-' } else { '+' }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_record(file_index: usize, chr: u8, conf: f64) -> MatchedRecord {
        MatchedRecord {
            file_index,
            ref_contig_id: chr,
            qry_start_pos: 0.0,
            qry_end_pos: 100.0,
            ref_start_pos: 1000.0,
            ref_end_pos: 1100.0,
            orientation: '+',
            confidence: conf,
            ref_len: 250_000_000.0,
        }
    }

    #[test]
    fn push_match_updates_columns_and_indexes() {
        let store = MatchStore::new();
        let recs = vec![mk_record(0, 1, 10.0), mk_record(1, 1, 11.0)];
        store.push_match(42, &[0, 1], &recs, &[0, 1]);

        let snap = store.progress_snapshot();
        assert_eq!(snap.total_matches, 1);
        assert_eq!(snap.total_records, 2);
        assert_eq!(snap.per_genome_records, vec![1, 1]);

        assert_eq!(store.distinct_contig_count(), 1);
    }

    #[test]
    fn empty_match_does_not_desync() {
        let store = MatchStore::new();
        store.push_match(42, &[], &[], &[]);
        let snap = store.progress_snapshot();
        assert_eq!(snap.total_matches, 0);
        assert_eq!(snap.total_records, 0);
    }

    #[test]
    fn orientation_roundtrip() {
        assert_eq!(decode_orientation(encode_orientation('+')), '+');
        assert_eq!(decode_orientation(encode_orientation('-')), '-');
        assert_eq!(decode_orientation(encode_orientation('?')), '+');
    }
}