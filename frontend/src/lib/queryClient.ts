// Server-side types mirror query.rs.

export interface ChromosomeCount {
  genome_index: number;
  chromosome: number;
  count: number;
}

export interface SequenceAggregate {
  qry_contig_id: number;
  total_occurrences: number;
  per_genome: number[];
  per_chromosome: ChromosomeCount[];
  max_confidence: number;
}

export interface MetaResponse {
  max_confidence: number;
  available_sequence_ids: number[];
  file_to_genome: number[];
  total_matches: number;
  total_records: number;
}

export interface MatchedRecordWire {
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

export interface MatchEntry {
  qry_contig_id: number;
  records: MatchedRecordWire[];
  total_record_count: number;
  records_truncated: boolean;
}

export interface SeqsPage {
  total: number;
  items: SequenceAggregate[];
}

export interface MatchesPage {
  total: number;
  items: MatchEntry[];
}

export type SearchType = 'sequence' | 'chromosome' | 'confidence';

const BASE = 'http://localhost:8080';

export class QueryError extends Error {
  constructor(public status: number, public statusText: string, msg?: string) {
    super(msg ?? `${status} ${statusText}`);
    this.name = 'QueryError';
  }
}

function isAbort(err: unknown): boolean {
  return err instanceof DOMException && err.name === 'AbortError';
}

async function fetchJSON<T>(url: string, signal?: AbortSignal): Promise<T | undefined> {
  try {
    const resp = await fetch(url, { signal });
    if (!resp.ok) throw new QueryError(resp.status, resp.statusText);
    return (await resp.json()) as T;
  } catch (err) {
    if (isAbort(err)) return undefined;
    throw err;
  }
}

async function fetchBincode(url: string, signal?: AbortSignal): Promise<Uint8Array | undefined> {
  try {
    const resp = await fetch(url, { signal });
    if (!resp.ok) throw new QueryError(resp.status, resp.statusText);
    const bytes = new Uint8Array(await resp.arrayBuffer());
    if (bytes.length < 4) throw new QueryError(500, 'Malformed bincode response (too short)');
    const bodyLen =
      bytes[0] | (bytes[1] << 8) | (bytes[2] << 16) | (bytes[3] << 24);
    if (bodyLen + 4 !== bytes.length) {
      console.warn(`bincode body length ${bodyLen} + 4 header != total ${bytes.length}`);
    }
    return bytes.subarray(4, 4 + bodyLen);
  } catch (err) {
    if (isAbort(err)) return undefined;
    throw err;
  }
}

function qs(params: Record<string, string | number | boolean | undefined | null>): string {
  const parts: string[] = [];
  for (const [k, v] of Object.entries(params)) {
    if (v === undefined || v === null || v === '') continue;
    parts.push(`${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`);
  }
  return parts.length ? `?${parts.join('&')}` : '';
}

// ---------------------------------------------------------------------------
// Public endpoint wrappers
// ---------------------------------------------------------------------------

export function fetchMeta(
  sessId: string,
  signal?: AbortSignal,
): Promise<MetaResponse | undefined> {
  return fetchJSON<MetaResponse>(`${BASE}/api/session/${sessId}/meta`, signal);
}

interface PageOpts {
  q?: string;
  searchType?: SearchType;
  page?: number;
  perPage?: number;
  signal?: AbortSignal;
}

function pageQs(opts: PageOpts): string {
  return qs({
    q: opts.q,
    search_type: opts.searchType,
    page: opts.page,
    per_page: opts.perPage,
  });
}

export function fetchSeqs(sessId: string, opts: PageOpts = {}): Promise<SeqsPage | undefined> {
  return fetchJSON<SeqsPage>(`${BASE}/api/session/${sessId}/sequences${pageQs(opts)}`, opts.signal);
}

export function fetchMatchesPage(
  sessId: string,
  opts: PageOpts = {},
): Promise<MatchesPage | undefined> {
  return fetchJSON<MatchesPage>(`${BASE}/api/session/${sessId}/matches${pageQs(opts)}`, opts.signal);
}

export interface WireFlow {
  qry_contig_id: number;
  from_genome: number;
  from_chromosome: number;
  from_orientation: string;
  from_confidence: number;
  to_genome: number;
  to_chromosome: number;
  to_orientation: string;
  to_confidence: number;
}

export async function fetchFlows(
  sessId: string,
  opts: {
    qry?: number;
    g1?: number;
    g2?: number;
    chr?: number;
    chrGen?: number;
    showDups?: boolean;
    limit?: number;
    signal?: AbortSignal;
  } = {},
): Promise<WireFlow[] | undefined> {
  const url = `${BASE}/api/session/${sessId}/flows${qs({
    qry: opts.qry,
    g1: opts.g1,
    g2: opts.g2,
    chr: opts.chr,
    chr_genome: opts.chrGen,
    show_duplicates: opts.showDups,
    limit: opts.limit,
  })}`;
  const bytes = await fetchBincode(url, opts.signal);
  return bytes ? decFlows(bytes) : undefined;
}

/**
 * bincode layout: u64 LE length, then per element:
 *   qry_contig_id     u32
 *   from_genome       u32
 *   from_chromosome   u8
 *   from_orientation  char (1..4 bytes UTF-8; ASCII in practice)
 *   from_confidence   f64
 *   to_genome         u32
 *   to_chromosome     u8
 *   to_orientation    char
 *   to_confidence     f64
 */
function decFlows(bytes: Uint8Array): WireFlow[] {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let pos = 0;
  const n = Number(view.getBigUint64(pos, true)); pos += 8;
  const out: WireFlow[] = new Array(n);

  for (let i = 0; i < n; i++) {
    const qry_contig_id = view.getUint32(pos, true); pos += 4;
    const from_genome = view.getUint32(pos, true); pos += 4;
    const from_chromosome = view.getUint8(pos); pos += 1;
    const from_orientation = readChar(view, pos); pos += charLen(view.getUint8(pos));
    const from_confidence = view.getFloat64(pos, true); pos += 8;

    const to_genome = view.getUint32(pos, true); pos += 4;
    const to_chromosome = view.getUint8(pos); pos += 1;
    const to_orientation = readChar(view, pos); pos += charLen(view.getUint8(pos));
    const to_confidence = view.getFloat64(pos, true); pos += 8;

    out[i] = {
      qry_contig_id,
      from_genome, from_chromosome, from_orientation, from_confidence,
      to_genome, to_chromosome, to_orientation, to_confidence,
    };
  }
  return out;
}

function readChar(view: DataView, pos: number): string {
  const b = view.getUint8(pos);
  return (b & 0x80) === 0 ? String.fromCharCode(b) : '?';
}

function charLen(lead: number): number {
  if ((lead & 0x80) === 0) return 1;
  if ((lead & 0xE0) === 0xC0) return 2;
  if ((lead & 0xF0) === 0xE0) return 3;
  if ((lead & 0xF8) === 0xF0) return 4;
  return 1;
}

// ---------------------------------------------------------------------------
// Chromosome-records endpoint (bincode) — for AreaAnalysis tab
// ---------------------------------------------------------------------------
export interface WireAreaRecord {
  qry_contig_id: number;
  file_index: number;
  genome_index: number;
  ref_contig_id: number;
  qry_start_pos: number;
  qry_end_pos: number;
  ref_start_pos: number;
  ref_end_pos: number;
  orientation: string;
  confidence: number;
  ref_len: number;
}

export interface ChrRecsResponse {
  chromosome: number;
  chromosome_ref_len: number;
  records: WireAreaRecord[];
}

export async function fetchChrRecs(
  sessId: string,
  opts: { genomes?: number[]; chr: number; qry?: number; signal?: AbortSignal },
): Promise<ChrRecsResponse | undefined> {
  const url = `${BASE}/api/session/${sessId}/chromosome-records${qs({
    genomes: opts.genomes && opts.genomes.length > 0 ? opts.genomes.join(',') : undefined,
    chr: opts.chr,
    qry: opts.qry,
  })}`;
  const bytes = await fetchBincode(url, opts.signal);
  return bytes ? decChrRecs(bytes) : undefined;
}

export interface SeqLocation {
  genome_index: number;
  ref_contig_id: number;
  ref_start_pos: number;
  ref_end_pos: number;
}

export interface SeqLocationsResp {
  qry_contig_id: number;
  locations: SeqLocation[];
}

export function fetchSeqLocations(
  sessId: string,
  opts: { qry: number; genomes?: number[]; signal?: AbortSignal },
): Promise<SeqLocationsResp | undefined> {
  const url = `${BASE}/api/session/${sessId}/sequence-locations${qs({
    qry: opts.qry,
    genomes: opts.genomes && opts.genomes.length > 0 ? opts.genomes.join(',') : undefined,
  })}`;
  return fetchJSON<SeqLocationsResp>(url, opts.signal);
}

/**
 * Wire layout:
 *   chromosome u8, chromosome_ref_len f64, records_len u64 LE,
 *   records: qry_contig_id u32, file_index u32, genome_index u32,
 *            ref_contig_id u8, qry_start/end f64, ref_start/end f64,
 *            orientation char, confidence f64, ref_len f64.
 */
function decChrRecs(bytes: Uint8Array): ChrRecsResponse {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let pos = 0;

  const chromosome = view.getUint8(pos); pos += 1;
  const chromosome_ref_len = view.getFloat64(pos, true); pos += 8;
  const n = Number(view.getBigUint64(pos, true)); pos += 8;

  const records: WireAreaRecord[] = new Array(n);
  for (let i = 0; i < n; i++) {
    const qry_contig_id = view.getUint32(pos, true); pos += 4;
    const file_index    = view.getUint32(pos, true); pos += 4;
    const genome_index  = view.getUint32(pos, true); pos += 4;
    const ref_contig_id = view.getUint8(pos);        pos += 1;
    const qry_start_pos = view.getFloat64(pos, true); pos += 8;
    const qry_end_pos   = view.getFloat64(pos, true); pos += 8;
    const ref_start_pos = view.getFloat64(pos, true); pos += 8;
    const ref_end_pos   = view.getFloat64(pos, true); pos += 8;
    const orientation   = readChar(view, pos); pos += charLen(view.getUint8(pos));
    const confidence    = view.getFloat64(pos, true); pos += 8;
    const ref_len       = view.getFloat64(pos, true); pos += 8;

    records[i] = {
      qry_contig_id, file_index, genome_index, ref_contig_id,
      qry_start_pos, qry_end_pos, ref_start_pos, ref_end_pos,
      orientation, confidence, ref_len,
    };
  }

  return { chromosome, chromosome_ref_len, records };
}

export function makeDebouncer(ms: number) {
  let timer: ReturnType<typeof setTimeout> | null = null;
  return {
    schedule(fn: () => void) {
      if (timer !== null) clearTimeout(timer);
      timer = setTimeout(() => { timer = null; fn(); }, ms);
    },
    cancel() {
      if (timer !== null) { clearTimeout(timer); timer = null; }
    },
  };
}