import { describe, it, expect } from 'vitest';
import { __testUtils } from '../src/lib/bincodeDecoder';

const { decodeBackendMatch } = __testUtils;

/**
 * Helper to write little-endian 32-bit and 64-bit numbers into a Uint8Array
 */
function writeU32(value: number) {
  const buf = new Uint8Array(4);
  buf[0] = value & 0xff;
  buf[1] = (value >> 8) & 0xff;
  buf[2] = (value >> 16) & 0xff;
  buf[3] = (value >> 24) & 0xff;
  return buf;
}

function writeU64(value: bigint | number) {
  const v = BigInt(value);
  const buf = new Uint8Array(8);
  for (let i = 0n; i < 8n; i++) {
    buf[Number(i)] = Number((v >> (8n * i)) & 0xffn);
  }
  return buf;
}

function writeF64(value: number) {
  const buf = new ArrayBuffer(8);
  new DataView(buf).setFloat64(0, value, true);
  return new Uint8Array(buf);
}

function writeChar(ch: string) {
  return new TextEncoder().encode(ch);
}

/**
 * Builds a realistic encoded BackendMatch with one record
 */
function buildMockBinary() {
  const parts: Uint8Array[] = [];

  // qry_contig_id (U32)
  parts.push(writeU32(2001));

  // file_indices_len (U64) = 1
  parts.push(writeU64(1n));
  // file_indices[0] = 0
  parts.push(writeU64(0n));

  // records_len (U64) = 1
  parts.push(writeU64(1n));

  // MatchedRecord
  parts.push(writeU64(0n)); // file_index
  parts.push(new Uint8Array([1])); // ref_contig_id
  parts.push(writeF64(1000.0)); // qry_start_pos
  parts.push(writeF64(5000.0)); // qry_end_pos
  parts.push(writeF64(0.0)); // ref_start_pos
  parts.push(writeF64(250000.0)); // ref_end_pos
  parts.push(writeChar('+')); // orientation
  parts.push(writeF64(9.8)); // confidence
  parts.push(writeF64(250000.0)); // ref_len

  const totalLength = parts.reduce((acc, arr) => acc + arr.length, 0);
  const buffer = new Uint8Array(totalLength);
  let offset = 0;
  for (const arr of parts) {
    buffer.set(arr, offset);
    offset += arr.length;
  }
  return buffer;
}

describe('decodeBackendMatch', () => {
  it('decodes a mock binary buffer correctly', () => {
    const binary = buildMockBinary();
    const match = decodeBackendMatch(binary);
    expect(match.qry_contig_id).toBe(2001);
    expect(match.file_indices).toEqual([0]);
    expect(match.records.length).toBe(1);

    const record = match.records[0];
    expect(record.ref_contig_id).toBe(1);
    expect(record.qry_start_pos).toBeCloseTo(1000.0);
    expect(record.qry_end_pos).toBeCloseTo(5000.0);
    expect(record.ref_end_pos).toBeCloseTo(250000.0);
    expect(record.orientation).toBe('+');
    expect(record.confidence).toBeCloseTo(9.8);
    expect(record.ref_len).toBeCloseTo(250000.0);
  });
});
