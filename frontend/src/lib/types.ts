export interface FileData {
  name: string;
  rows: number;
  color: string;
}

// ---------------------------------------------------------------------------
// Wire-format records (mirror of bincodeDecoder equivalents)
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
// Donut-chart geometry types (derived from the above)
// ---------------------------------------------------------------------------

export interface DonutSegment {
  name: string;
  rows: number;
  color: string;
  index: number;
  genomeSize: number;
  dashArray: string;
  dashOffset: number;
  percentage: string;
  showLabel: boolean;
  showChromosomes: boolean;
  startAngle: number;
  endAngle: number;
  angleRange: number;
}

export interface FlowPath {
  path: string;
  p1: { x: number; y: number };
  p2: { x: number; y: number };
  fromOrientation: string;
  toOrientation: string;
  color: string;
  opacity: number;
  width: number;
  fromChromosome: number;
  toChromosome: number;
  confidence: number;
  fromFileIndex: number;
  toFileIndex: number;
  isSameGenome: boolean;
  qryContigId: number;
  fromRecord: MatchedRecord;
  toRecord: MatchedRecord;
}

export interface ChromosomeDivision {
  chromosome: number;
  startAngle: number;
  endAngle: number;
  midAngle: number;
}