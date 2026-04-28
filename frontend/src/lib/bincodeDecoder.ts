export interface ChromosomeInfo {
  ref_contig_id: number;
  ref_len: number;
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
  distinct_sequence_count: number;
}

export type StreamFrame =
  | { type: 'chromosomeInfo'; chromosomeInfo: ChromosomeInfo[][] }
  | { type: 'progress';       progress:       ProgressFrame      }
  | { type: 'complete';       complete:       CompleteFrame      };

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
}

function decChrInfoOuter(rd: ByteReader): ChromosomeInfo[][] {
  const n = Number(rd.readU64());
  const out: ChromosomeInfo[][] = [];
  for (let i = 0; i < n; i++) {
    const len = Number(rd.readU64());
    const inner: ChromosomeInfo[] = [];
    for (let j = 0; j < len; j++) {
      const ref_contig_id = rd.readU8();
      const ref_len = rd.readF64();
      inner.push({ ref_contig_id, ref_len });
    }
    out.push(inner);
  }
  return out;
}

function decU64Vec(rd: ByteReader): number[] {
  const n = Number(rd.readU64());
  const out: number[] = [];
  for (let i = 0; i < n; i++) out.push(Number(rd.readU64()));
  return out;
}

function decStreamFrame(bytes: Uint8Array): StreamFrame {
  const rd = new ByteReader(bytes);
  const variant = rd.readU32();

  switch (variant) {
    case 0:
      return { type: 'chromosomeInfo', chromosomeInfo: decChrInfoOuter(rd) };
    case 1:
      return {
        type: 'progress',
        progress: {
          total_matches:      Number(rd.readU64()),
          total_records:      Number(rd.readU64()),
          per_genome_records: decU64Vec(rd),
        },
      };
    case 2:
      return {
        type: 'complete',
        complete: {
          total_matches:           Number(rd.readU64()),
          total_records:           Number(rd.readU64()),
          per_genome_records:      decU64Vec(rd),
          distinct_sequence_count: Number(rd.readU64()),
        },
      };
    default:
      throw new Error(`Unknown StreamFrame variant index: ${variant}`);
  }
}

export async function* processMatchStream(
  resp: Response,
): AsyncGenerator<StreamFrame> {
  const rd = resp.body?.getReader();
  if (!rd) throw new Error('No response body available');

  let buf = new Uint8Array(1 << 14); // 16 KiB initial
  let readPos = 0;
  let writePos = 0;
  let msgCnt = 0;

  const ensureCap = (extra: number) => {
    if (writePos + extra <= buf.length) return;
    if (readPos > 0) {
      buf.copyWithin(0, readPos, writePos);
      writePos -= readPos;
      readPos = 0;
      if (writePos + extra <= buf.length) return;
    }
    let next = buf.length;
    while (next < writePos + extra) next = Math.max(next * 2, next + extra);
    const grown = new Uint8Array(next);
    grown.set(buf.subarray(0, writePos));
    buf = grown;
  };

  try {
    while (true) {
      const { done, value } = await rd.read();
      if (done) {
        const left = writePos - readPos;
        if (left > 0) console.warn(`Leftover bytes: ${left}`);
        break;
      }

      ensureCap(value.length);
      buf.set(value, writePos);
      writePos += value.length;

      while (writePos - readPos >= 4) {
        const len =
          buf[readPos] |
          (buf[readPos + 1] << 8) |
          (buf[readPos + 2] << 16) |
          (buf[readPos + 3] << 24);

        if (len < 0 || len > 10_000_000) {
          console.warn(`Suspicious frame length: ${len}`);
        }
        if (writePos - readPos < 4 + len) break;

        const msgBytes = buf.subarray(readPos + 4, readPos + 4 + len);
        readPos += 4 + len;

        try {
          const frame = decStreamFrame(msgBytes);
          msgCnt++;
          yield frame;
        } catch (err) {
          console.error(`Failed to decode frame ${msgCnt + 1}:`, err);
        }
      }

      if (readPos > buf.length >>> 1) {
        buf.copyWithin(0, readPos, writePos);
        writePos -= readPos;
        readPos = 0;
      }
    }
  } finally {
    rd.releaseLock();
  }
}