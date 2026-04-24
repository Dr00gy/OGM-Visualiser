export interface ChromosomeInfo {
  ref_contig_id: number;
  ref_len: number;
}

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

export interface RecordChunk {
  qry_contig_id: number;
  file_indices: number[];
  records: MatchedRecord[];
  chunk_index: number;
  total_chunks: number;
}

export interface ProgressFrame {
  total_matches: number;
  total_records: number;
  per_genome_records: number[];
}

export interface CompleteFrame {
  total_matches: number;
  total_records: number;
  per_genome_records: number[];
  distinct_contig_count: number;
}

export type StreamFrame =
  | { type: 'chromosomeInfo'; chromosomeInfo: ChromosomeInfo[][] }
  | { type: 'progress';       progress:       ProgressFrame      }
  | { type: 'complete';       complete:       CompleteFrame      };

// ---------------------------------------------------------------------------
// ByteReader: forward-only reader over a Uint8Array
// ---------------------------------------------------------------------------

class ByteReader {
  private data: Uint8Array;
  private view: DataView;
  private pos: number;

  constructor(data: Uint8Array) {
    this.data = data;
    this.view = new DataView(data.buffer, data.byteOffset, data.byteLength);
    this.pos = 0;
  }

  readU8(): number {
    if (this.pos >= this.data.length) {
      throw new Error(`Read past end: pos=${this.pos}, len=${this.data.length}`);
    }
    const v = this.view.getUint8(this.pos);
    this.pos += 1;
    return v;
  }

  readU32(): number {
    if (this.pos + 4 > this.data.length) {
      throw new Error(`readU32 out of range at ${this.pos}`);
    }
    const v = this.view.getUint32(this.pos, true);
    this.pos += 4;
    return v;
  }

  readU64(): bigint {
    if (this.pos + 8 > this.data.length) {
      throw new Error(`readU64 out of range at ${this.pos}`);
    }
    const v = this.view.getBigUint64(this.pos, true);
    this.pos += 8;
    return v;
  }

  readF64(): number {
    if (this.pos + 8 > this.data.length) {
      throw new Error(`readF64 out of range at ${this.pos}`);
    }
    const v = this.view.getFloat64(this.pos, true);
    this.pos += 8;
    return v;
  }

  remaining(): number { return this.data.length - this.pos; }
  getPos(): number    { return this.pos; }
}

// ---------------------------------------------------------------------------
// Per-variant decoders
// ---------------------------------------------------------------------------

function decodeChromosomeInfoVec(reader: ByteReader): ChromosomeInfo[] {
  const length = Number(reader.readU64());
  const chromosomes: ChromosomeInfo[] = [];
  for (let i = 0; i < length; i++) {
    const ref_contig_id = reader.readU8();
    const ref_len = reader.readF64();
    chromosomes.push({ ref_contig_id, ref_len });
  }
  return chromosomes;
}

function decodeChromosomeInfoOuter(reader: ByteReader): ChromosomeInfo[][] {
  const n = Number(reader.readU64());
  const out: ChromosomeInfo[][] = [];
  for (let i = 0; i < n; i++) {
    out.push(decodeChromosomeInfoVec(reader));
  }
  return out;
}

function decodeU64Vec(reader: ByteReader): number[] {
  const n = Number(reader.readU64());
  const out: number[] = [];
  for (let i = 0; i < n; i++) {
    out.push(Number(reader.readU64()));
  }
  return out;
}

function decodeProgressFrame(reader: ByteReader): ProgressFrame {
  const total_matches       = Number(reader.readU64());
  const total_records       = Number(reader.readU64());
  const per_genome_records  = decodeU64Vec(reader);
  return { total_matches, total_records, per_genome_records };
}

function decodeCompleteFrame(reader: ByteReader): CompleteFrame {
  const total_matches          = Number(reader.readU64());
  const total_records          = Number(reader.readU64());
  const per_genome_records     = decodeU64Vec(reader);
  const distinct_contig_count  = Number(reader.readU64());
  return { total_matches, total_records, per_genome_records, distinct_contig_count };
}

function decodeStreamFrame(bytes: Uint8Array): StreamFrame {
  const reader = new ByteReader(bytes);
  const variant = reader.readU32();

  switch (variant) {
    case 0: {
      const chromosomeInfo = decodeChromosomeInfoOuter(reader);
      return { type: 'chromosomeInfo', chromosomeInfo };
    }
    case 1: {
      const progress = decodeProgressFrame(reader);
      return { type: 'progress', progress };
    }
    case 2: {
      const complete = decodeCompleteFrame(reader);
      return { type: 'complete', complete };
    }
    default:
      throw new Error(`Unknown StreamFrame variant index: ${variant}`);
  }
}

// ---------------------------------------------------------------------------
// Streaming decoder (public entry point)
// ---------------------------------------------------------------------------

export async function* processMatchStream(
  response: Response
): AsyncGenerator<StreamFrame> {
  const reader = response.body?.getReader();
  if (!reader) {
    throw new Error('No response body available');
  }

  let buffer = new Uint8Array(1 << 14); // 16 KiB initial
  let readPos = 0;
  let writePos = 0;
  let messageCount = 0;

  const ensureCapacity = (additional: number) => {
    if (writePos + additional <= buffer.length) return;
    if (readPos > 0) {
      buffer.copyWithin(0, readPos, writePos);
      writePos -= readPos;
      readPos = 0;
      if (writePos + additional <= buffer.length) return;
    }
    let newSize = buffer.length;
    while (newSize < writePos + additional) {
      newSize = Math.max(newSize * 2, newSize + additional);
    }
    const next = new Uint8Array(newSize);
    next.set(buffer.subarray(0, writePos));
    buffer = next;
  };

  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        const leftover = writePos - readPos;
        if (leftover > 0) {
          console.warn(`Leftover bytes: ${leftover}`);
        }
        break;
      }

      ensureCapacity(value.length);
      buffer.set(value, writePos);
      writePos += value.length;

      while (writePos - readPos >= 4) {
        const length =
          buffer[readPos] |
          (buffer[readPos + 1] << 8) |
          (buffer[readPos + 2] << 16) |
          (buffer[readPos + 3] << 24);

        if (length < 0 || length > 10_000_000) {
          console.warn(`Suspicious frame length: ${length}`);
        }
        if (writePos - readPos < 4 + length) {
          break;
        }

        const messageBytes = buffer.subarray(readPos + 4, readPos + 4 + length);
        readPos += 4 + length;

        try {
          const frame = decodeStreamFrame(messageBytes);
          messageCount++;
          yield frame;
        } catch (error) {
          console.error(`Failed to decode frame ${messageCount + 1}:`, error);
        }
      }

      if (readPos > buffer.length >>> 1) {
        buffer.copyWithin(0, readPos, writePos);
        writePos -= readPos;
        readPos = 0;
      }
    }
  } finally {
    reader.releaseLock();
  }
}

// ---------------------------------------------------------------------------
// Test hooks — internal, not for application use.
// ---------------------------------------------------------------------------

export const __testUtils = {
  decodeStreamFrame,
  decodeChromosomeInfoOuter,
  decodeProgressFrame,
  decodeCompleteFrame,
};