use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::AppState;
use crate::store::{SequenceAggregate, decode_orientation};

const MAX_RECORDS_PER_ENTRY: usize = 50;
const FLOW_LIMIT_CAP: u32 = 1_000_000;

fn resolve_session(
    state: &AppState,
    session_id: &str,
) -> Result<(Arc<crate::store::MatchStore>, Vec<usize>), StatusCode> {
    let uuid = Uuid::parse_str(session_id).map_err(|_| StatusCode::NOT_FOUND)?;
    let entry = state.sessions.get(&uuid).ok_or(StatusCode::NOT_FOUND)?;
    if !entry.match_complete {
        return Err(StatusCode::CONFLICT);
    }
    let store = Arc::clone(&entry.store);
    let file_to_genome = entry.file_to_genome.clone().unwrap_or_default();
    Ok((store, file_to_genome))
}

fn parse_genome_csv(s: Option<&str>) -> Option<Vec<u32>> {
    let s = s?;
    if s.is_empty() { return None; }
    let parsed: Vec<u32> = s
        .split(',')
        .filter_map(|tok| tok.trim().parse::<u32>().ok())
        .collect();
    if parsed.is_empty() { None } else { Some(parsed) }
}

fn framed_bincode<T: Serialize>(value: &T) -> Result<Response, StatusCode> {
    let bytes = bincode::serialize(value).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut framed = Vec::with_capacity(4 + bytes.len());
    framed.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    framed.extend_from_slice(&bytes);
    Ok((
        [(header::CONTENT_TYPE, "application/octet-stream")],
        Body::from(framed),
    ).into_response())
}

/// Predicate over a `SequenceAggregate` based on (search_type, lowercase needle).
/// Used by both `/sequences` and `/matches`. The `chr_match_includes_genome`
/// flag controls whether the chromosome filter accepts `"<g>-<c>"` (sequences
/// page) or just the chromosome number (matches page).
fn matches_filter(
    agg: &SequenceAggregate,
    search_type: &str,
    needle: &str,
    chr_match_includes_genome: bool,
) -> bool {
    if needle.is_empty() { return true; }
    match search_type {
        "sequence" => agg.qry_contig_id.to_string().contains(needle),
        "chromosome" => {
            if chr_match_includes_genome {
                agg.per_chromosome.iter().any(|c| {
                    format!("{}-{}", c.genome_index, c.chromosome).contains(needle)
                })
            } else {
                agg.per_chromosome.iter().any(|c| c.chromosome.to_string().contains(needle))
            }
        }
        "confidence" => format!("{:.2}", agg.max_confidence).contains(needle),
        _ => true,
    }
}

#[derive(Debug, Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub q: String,
    #[serde(default = "default_search_type")]
    pub search_type: String,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_per_page")]
    pub per_page: u32,
}

fn default_search_type() -> String { "sequence".to_string() }
fn default_page() -> u32 { 1 }
fn default_per_page() -> u32 { 10 }

fn page_bounds(page: u32, per_page: u32) -> (usize, usize) {
    let per_page = per_page.min(200).max(1) as usize;
    let page = page.max(1) as usize;
    let start = (page - 1).saturating_mul(per_page);
    (start, per_page)
}

// ---------------------------------------------------------------------------
// /meta
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct MetaResponse {
    pub max_confidence: f64,
    pub available_sequence_ids: Vec<u32>,
    pub file_to_genome: Vec<u32>,
    pub total_matches: u64,
    pub total_records: u64,
}

pub async fn get_meta(
    State(state): State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> Result<Response, StatusCode> {
    let (store, file_to_genome) = resolve_session(&state, &session_id)?;
    let snap = store.snapshot();
    let meta = MetaResponse {
        max_confidence: store.max_confidence(),
        available_sequence_ids: store.available_sequence_ids(),
        file_to_genome: file_to_genome.into_iter().map(|g| g as u32).collect(),
        total_matches: snap.total_matches,
        total_records: snap.total_records,
    };
    framed_bincode(&meta)
}

// ---------------------------------------------------------------------------
// /sequences
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SequencesPage {
    pub total: u64,
    pub items: Vec<SequenceAggregate>,
}

pub async fn get_sequences(
    State(state): State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Query(params): Query<PageQuery>,
) -> Result<Response, StatusCode> {
    let (store, _) = resolve_session(&state, &session_id)?;
    let (start, per_page) = page_bounds(params.page, params.per_page);
    let needle = params.q.to_ascii_lowercase();

    let (total, items) = store.scan_and_paginate(start, per_page, |agg| {
        matches_filter(agg, &params.search_type, &needle, true)
    });

    framed_bincode(&SequencesPage { total, items })
}

// ---------------------------------------------------------------------------
// /matches
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct WireRecord {
    pub file_index: u32,
    pub ref_contig_id: u8,
    pub qry_start_pos: f64,
    pub qry_end_pos: f64,
    pub ref_start_pos: f64,
    pub ref_end_pos: f64,
    pub orientation: char,
    pub confidence: f64,
    pub ref_len: f64,
}

#[derive(Debug, Serialize)]
pub struct MatchEntry {
    pub qry_contig_id: u32,
    pub records: Vec<WireRecord>,
    pub total_record_count: u32,
    pub records_truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct MatchesPage {
    pub total: u64,
    pub items: Vec<MatchEntry>,
}

pub async fn get_matches(
    State(state): State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Query(params): Query<PageQuery>,
) -> Result<Response, StatusCode> {
    let (store, _) = resolve_session(&state, &session_id)?;
    let (start, per_page) = page_bounds(params.page, params.per_page);
    let needle = params.q.to_ascii_lowercase();

    let (total, page_aggs) = store.scan_and_paginate(start, per_page, |agg| {
        matches_filter(agg, &params.search_type, &needle, false)
    });

    if page_aggs.is_empty() {
        return framed_bincode(&MatchesPage { total, items: Vec::new() });
    }

    let page_sequence_ids: Vec<u32> = page_aggs.iter().map(|a| a.qry_contig_id).collect();
    let items: Vec<MatchEntry> = store.with_read(|inner| {
        use std::collections::HashSet;
        let mut out: Vec<MatchEntry> = Vec::with_capacity(page_sequence_ids.len());
        for &qry_id in &page_sequence_ids {
            let Some(rows) = inner.rows_by_sequence.get(&qry_id) else { continue; };

            let mut seen: HashSet<(u32, u8, u8, u64)> = HashSet::new();
            let mut records: Vec<WireRecord> = Vec::new();

            for &row in rows {
                let ri = row as usize;
                let file_index = inner.file_index[ri];
                let chr = inner.ref_contig_id[ri];
                let orient_byte = inner.orientation[ri];
                let conf_bits = inner.confidence[ri].to_bits();
                let key = (file_index, chr, orient_byte, conf_bits);
                if !seen.insert(key) { continue; }

                records.push(WireRecord {
                    file_index,
                    ref_contig_id: chr,
                    qry_start_pos: inner.qry_start_pos[ri],
                    qry_end_pos:   inner.qry_end_pos[ri],
                    ref_start_pos: inner.ref_start_pos[ri],
                    ref_end_pos:   inner.ref_end_pos[ri],
                    orientation:   decode_orientation(orient_byte),
                    confidence:    inner.confidence[ri],
                    ref_len:       inner.ref_len[ri],
                });
            }

            records.sort_by(|a, b| {
                a.file_index.cmp(&b.file_index)
                    .then_with(|| a.ref_contig_id.cmp(&b.ref_contig_id))
            });

            let total_record_count = records.len() as u32;
            let records_truncated = records.len() > MAX_RECORDS_PER_ENTRY;
            if records_truncated { records.truncate(MAX_RECORDS_PER_ENTRY); }

            out.push(MatchEntry {
                qry_contig_id: qry_id,
                records,
                total_record_count,
                records_truncated,
            });
        }
        out
    });

    framed_bincode(&MatchesPage { total, items })
}

// ---------------------------------------------------------------------------
// /flows
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct FlowsQuery {
    pub qry: Option<u32>,
    pub g1: Option<u32>,
    pub g2: Option<u32>,
    pub chr: Option<u8>,
    pub chr_genome: Option<u32>,
    #[serde(default)]
    pub show_duplicates: bool,
    #[serde(default = "default_flow_limit")]
    pub limit: u32,
}

fn default_flow_limit() -> u32 { 100_000 }

#[derive(Debug, Serialize)]
pub struct WireFlow {
    pub qry_contig_id: u32,
    pub from_genome: u32,
    pub from_chromosome: u8,
    pub from_orientation: char,
    pub from_confidence: f64,
    pub to_genome: u32,
    pub to_chromosome: u8,
    pub to_orientation: char,
    pub to_confidence: f64,
}

pub async fn get_flows(
    State(state): State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Query(params): Query<FlowsQuery>,
) -> Result<Response, StatusCode> {
    let (store, file_to_genome) = resolve_session(&state, &session_id)?;
    let want_same_genome = params.show_duplicates;
    let limit = params.limit.min(FLOW_LIMIT_CAP) as usize;

    let flows: Vec<WireFlow> = store.with_read(|inner| {
        use std::collections::HashSet;
        let mut out: Vec<WireFlow> = Vec::with_capacity(limit.min(65536));
        let mut seen_self: HashSet<(u32, u32, u8, u8)> = HashSet::new();

        'outer: for (sequence_id, rows) in inner.rows_by_sequence.iter() {
            if out.len() >= limit { break; }
            if rows.len() < 2 { continue; }
            if let Some(want) = params.qry {
                if *sequence_id != want { continue; }
            }

            for i in 0..rows.len() {
                for j in (i + 1)..rows.len() {
                    if out.len() >= limit { break 'outer; }

                    let ri = rows[i] as usize;
                    let rj = rows[j] as usize;

                    let from_g = file_to_genome.get(inner.file_index[ri] as usize).copied().unwrap_or(0) as u32;
                    let to_g   = file_to_genome.get(inner.file_index[rj] as usize).copied().unwrap_or(0) as u32;

                    let same_genome = from_g == to_g;
                    if same_genome != want_same_genome { continue; }

                    if let (Some(a), Some(b)) = (params.g1, params.g2) {
                        let ok = (from_g == a && to_g == b) || (from_g == b && to_g == a);
                        if !ok { continue; }
                    } else if let Some(a) = params.g1 {
                        if from_g != a && to_g != a { continue; }
                    }

                    let from_chr = inner.ref_contig_id[ri];
                    let to_chr   = inner.ref_contig_id[rj];

                    if let (Some(want_chr), Some(want_cg)) = (params.chr, params.chr_genome) {
                        let ok = (from_g == want_cg && from_chr == want_chr)
                            || (to_g   == want_cg && to_chr   == want_chr);
                        if !ok { continue; }
                    }

                    if same_genome {
                        let lo = from_chr.min(to_chr);
                        let hi = from_chr.max(to_chr);
                        if !seen_self.insert((*sequence_id, from_g, lo, hi)) { continue; }
                    }

                    out.push(WireFlow {
                        qry_contig_id: *sequence_id,
                        from_genome: from_g,
                        from_chromosome: from_chr,
                        from_orientation: decode_orientation(inner.orientation[ri]),
                        from_confidence: inner.confidence[ri],
                        to_genome: to_g,
                        to_chromosome: to_chr,
                        to_orientation: decode_orientation(inner.orientation[rj]),
                        to_confidence: inner.confidence[rj],
                    });
                }
            }
        }

        out
    });

    framed_bincode(&flows)
}

// ---------------------------------------------------------------------------
// /sequence-locations
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SequenceLocationsQuery {
    pub qry: u32,
    pub genomes: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SequenceLocation {
    pub genome_index: u32,
    pub ref_contig_id: u8,
    pub ref_start_pos: f64,
    pub ref_end_pos: f64,
}

#[derive(Debug, Serialize)]
pub struct SequenceLocationsResponse {
    pub qry_contig_id: u32,
    pub locations: Vec<SequenceLocation>,
}

pub async fn get_sequence_locations(
    State(state): State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Query(params): Query<SequenceLocationsQuery>,
) -> Result<Response, StatusCode> {
    let (store, file_to_genome) = resolve_session(&state, &session_id)?;
    let want = params.qry;
    let genome_filter = parse_genome_csv(params.genomes.as_deref());

    let locations = store.with_read(|inner| {
        let mut out: Vec<SequenceLocation> = Vec::new();
        let Some(rows) = inner.rows_by_sequence.get(&want) else { return out; };
        for &row in rows {
            let ri = row as usize;
            let gi = file_to_genome.get(inner.file_index[ri] as usize).copied().unwrap_or(0) as u32;
            if let Some(ref g) = genome_filter {
                if !g.contains(&gi) { continue; }
            }
            out.push(SequenceLocation {
                genome_index: gi,
                ref_contig_id: inner.ref_contig_id[ri],
                ref_start_pos: inner.ref_start_pos[ri],
                ref_end_pos:   inner.ref_end_pos[ri],
            });
        }
        out
    });

    framed_bincode(&SequenceLocationsResponse { qry_contig_id: want, locations })
}

// ---------------------------------------------------------------------------
// /chromosome-records
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ChromosomeRecordsQuery {
    pub genomes: Option<String>,
    pub chr: u8,
    pub qry: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct WireAreaRecord {
    pub qry_contig_id: u32,
    pub file_index: u32,
    pub genome_index: u32,
    pub ref_contig_id: u8,
    pub qry_start_pos: f64,
    pub qry_end_pos: f64,
    pub ref_start_pos: f64,
    pub ref_end_pos: f64,
    pub orientation: char,
    pub confidence: f64,
    pub ref_len: f64,
}

#[derive(Debug, Serialize)]
pub struct ChromosomeRecordsResponse {
    pub chromosome: u8,
    pub chromosome_ref_len: f64,
    pub records: Vec<WireAreaRecord>,
}

pub async fn get_chromosome_records(
    State(state): State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Query(params): Query<ChromosomeRecordsQuery>,
) -> Result<Response, StatusCode> {
    let (store, file_to_genome) = resolve_session(&state, &session_id)?;
    let chr = params.chr;
    let qry_filter = params.qry;
    let genome_filter = parse_genome_csv(params.genomes.as_deref());

    let response = store.with_read(|inner| {
        use std::collections::HashSet;
        let mut seen: HashSet<(u32, u64, u64)> = HashSet::new();
        let mut out: Vec<WireAreaRecord> = Vec::new();
        let mut chr_ref_len: f64 = 0.0;

        for ri in 0..inner.ref_contig_id.len() {
            if inner.ref_contig_id[ri] != chr { continue; }

            let gi = file_to_genome.get(inner.file_index[ri] as usize).copied().unwrap_or(0) as u32;
            if let Some(ref want) = genome_filter {
                if !want.contains(&gi) { continue; }
            }

            let qry_contig_id = inner.qry_contig_id[ri];
            if let Some(q) = qry_filter {
                if qry_contig_id != q { continue; }
            }

            let key = (qry_contig_id, inner.ref_start_pos[ri].to_bits(), inner.ref_end_pos[ri].to_bits());
            if !seen.insert(key) { continue; }

            let ref_len = inner.ref_len[ri];
            if chr_ref_len == 0.0 { chr_ref_len = ref_len; }

            out.push(WireAreaRecord {
                qry_contig_id,
                file_index: inner.file_index[ri],
                genome_index: gi,
                ref_contig_id: chr,
                qry_start_pos: inner.qry_start_pos[ri],
                qry_end_pos:   inner.qry_end_pos[ri],
                ref_start_pos: inner.ref_start_pos[ri],
                ref_end_pos:   inner.ref_end_pos[ri],
                orientation:   decode_orientation(inner.orientation[ri]),
                confidence:    inner.confidence[ri],
                ref_len,
            });
        }

        ChromosomeRecordsResponse { chromosome: chr, chromosome_ref_len: chr_ref_len, records: out }
    });

    framed_bincode(&response)
}