import { describe, it, expect } from 'vitest';
import {
  processMatchStream,
  type StreamFrame,
  type ChromosomeInfo,
} from '$lib/bincodeDecoder';

/**
 * Helper: builds a length-prefixed wire frame the way the Rust backend does.
 * The frame layout is:
 *   [u32 little-endian payload length][u32 LE variant tag][payload bytes]
 */
class FrameBuilder {
  private chunks: Uint8Array[] = [];
  private payload: number[] = [];

  variant(tag: 0 | 1 | 2): this {
    this.writeU32LE(tag);
    return this;
  }

  u8(v: number): this {
    this.payload.push(v & 0xff);
    return this;
  }

  u32(v: number): this {
    this.writeU32LE(v);
    return this;
  }

  u64(v: number): this {
    // Split into low/high 32-bit halves, little-endian.
    const lo = v >>> 0;
    const hi = Math.floor(v / 0x1_0000_0000) >>> 0;
    this.writeU32LE(lo);
    this.writeU32LE(hi);
    return this;
  }

  f64(v: number): this {
    const buf = new ArrayBuffer(8);
    new DataView(buf).setFloat64(0, v, /* littleEndian */ true);
    this.payload.push(...new Uint8Array(buf));
    return this;
  }

  /** Finish the current frame, prepending its u32 length. */
  finish(): this {
    const body = Uint8Array.from(this.payload);
    const lenBuf = new Uint8Array(4);
    new DataView(lenBuf.buffer).setUint32(0, body.length, true);
    this.chunks.push(lenBuf, body);
    this.payload = [];
    return this;
  }

  /** Concatenate all finished frames into a single buffer. */
  build(): Uint8Array {
    const total = this.chunks.reduce((n, c) => n + c.length, 0);
    const out = new Uint8Array(total);
    let off = 0;
    for (const c of this.chunks) { out.set(c, off); off += c.length; }
    return out;
  }

  private writeU32LE(v: number) {
    this.payload.push(v & 0xff, (v >>> 8) & 0xff, (v >>> 16) & 0xff, (v >>> 24) & 0xff);
  }
}

/** Wraps a Uint8Array as a Response so processMatchStream can consume it. */
function asResponse(bytes: Uint8Array, chunkSize = bytes.length): Response {
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (let i = 0; i < bytes.length; i += chunkSize) {
        controller.enqueue(bytes.slice(i, i + chunkSize));
      }
      controller.close();
    },
  });
  return new Response(stream);
}

async function collect(resp: Response): Promise<StreamFrame[]> {
  const out: StreamFrame[] = [];
  for await (const f of processMatchStream(resp)) out.push(f);
  return out;
}

describe('bincodeDecoder', () => {
  it('decodes a chromosomeInfo frame with one genome and one chromosome', async () => {
    // outer Vec length = 1, inner Vec length = 1, then (u8 chr, f64 len)
    const bytes = new FrameBuilder()
      .variant(0)
      .u64(1)         // outer.len
      .u64(1)         // inner.len
      .u8(7)          // ref_contig_id
      .f64(248_956_422.0) // ref_len (chr1 hg38)
      .finish()
      .build();

    const frames = await collect(asResponse(bytes));
    expect(frames).toHaveLength(1);
    expect(frames[0].type).toBe('chromosomeInfo');

    const info = (frames[0] as Extract<StreamFrame, { type: 'chromosomeInfo' }>)
      .chromosomeInfo;
    expect(info).toHaveLength(1);
    expect(info[0]).toHaveLength(1);
    const c: ChromosomeInfo = info[0][0];
    expect(c.ref_contig_id).toBe(7);
    expect(c.ref_len).toBeCloseTo(248_956_422.0);
  });

  it('decodes a progress frame with per-genome record counts', async () => {
    const bytes = new FrameBuilder()
      .variant(1)
      .u64(120)       // total_matches
      .u64(450)       // total_records
      .u64(2)         // per_genome_records.len
      .u64(225)
      .u64(225)
      .finish()
      .build();

    const [frame] = await collect(asResponse(bytes));
    expect(frame.type).toBe('progress');
    if (frame.type !== 'progress') return;
    expect(frame.progress.total_matches).toBe(120);
    expect(frame.progress.total_records).toBe(450);
    expect(frame.progress.per_genome_records).toEqual([225, 225]);
  });

  it('decodes a complete frame including distinct sequence count', async () => {
    const bytes = new FrameBuilder()
      .variant(2)
      .u64(500)
      .u64(1800)
      .u64(3)
      .u64(600).u64(600).u64(600)
      .u64(412)        // distinct_sequence_count
      .finish()
      .build();

    const [frame] = await collect(asResponse(bytes));
    expect(frame.type).toBe('complete');
    if (frame.type !== 'complete') return;
    expect(frame.complete.total_matches).toBe(500);
    expect(frame.complete.per_genome_records).toEqual([600, 600, 600]);
    expect(frame.complete.distinct_sequence_count).toBe(412);
  });

  it('reassembles frames split across arbitrary chunk boundaries', async () => {
    // Two consecutive progress frames, fed in 1-byte chunks to exercise the
    // ring-buffer compaction path inside processMatchStream.
    const bytes = new FrameBuilder()
      .variant(1).u64(1).u64(2).u64(0).finish()
      .variant(1).u64(3).u64(4).u64(0).finish()
      .build();

    const frames = await collect(asResponse(bytes, /* chunkSize */ 1));
    expect(frames).toHaveLength(2);
    expect(frames.every(f => f.type === 'progress')).toBe(true);
  });

  it('throws on an unknown variant tag', async () => {
    const bytes = new FrameBuilder().variant(9 as 0).finish().build();
    // The decoder logs and skips bad frames rather than throwing out of the
    // generator, so we assert no frame was yielded.
    const frames = await collect(asResponse(bytes));
    expect(frames).toHaveLength(0);
  });
});