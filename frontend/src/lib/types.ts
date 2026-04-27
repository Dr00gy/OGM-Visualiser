export interface FileData {
  name: string;
  rows: number;
  color: string;
}

// ---------------------------------------------------------------------------
// Wire-format records (mirror of bincodeDecoder equivalents).
// Field names match the BE Rust serialization byte-for-byte / key-for-key.
// ---------------------------------------------------------------------------

export interface MatchedRecord {
  file_index: number;
  ref_contig_id: number;
  qry_start_pos: number;
  qry_end_pos: number;
  ref_start_pos: number;
  ref_end_pos: number;
  orientation: string;
  confidence: number;
  ref_len: number;
}

export interface BackendMatch {
  qry_contig_id: number;
  file_indices: number[];
  records: MatchedRecord[];
}

// ---------------------------------------------------------------------------
// Donut-chart geometry types (FE-only, derived from the above)
// ---------------------------------------------------------------------------

export interface DonutSeg {
  name: string;
  rows: number;
  color: string;
  idx: number;
  genSize: number;
  dashArray: string;
  dashOffset: number;
  pct: string;
  showLabel: boolean;
  showChrs: boolean;
  startAng: number;
  endAng: number;
  angRange: number;
}

export interface FlowPath {
  path: string;
  p1: { x: number; y: number };
  p2: { x: number; y: number };
  fromOri: string;
  toOri: string;
  color: string;
  opacity: number;
  width: number;
  fromChr: number;
  toChr: number;
  conf: number;
  fromFileIdx: number;
  toFileIdx: number;
  isSameGen: boolean;
  qryContigId: number;
  fromRec: MatchedRecord;
  toRec: MatchedRecord;
}

export interface ChrDivision {
  chr: number;
  startAng: number;
  endAng: number;
  midAng: number;
}