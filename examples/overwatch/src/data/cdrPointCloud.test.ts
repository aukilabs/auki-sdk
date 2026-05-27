import { describe, it, expect } from "vitest";
import { decodePointCloud2 } from "./cdrPointCloud";

// Synthetic-frame helper: build a minimal CDR-encoded `PointCloud2`
// with the given xyz points. Match the schema layout the K1's
// StereoNetNode produces — 16-byte point_step (xyz FLOAT32 + 4 bytes
// of padding), header.frame_id = "test_frame", height = 1.
//
// Mirrors the encoder side of `decodePointCloud2`: 4-byte CDR LE
// encapsulation header, then the body with body-relative alignment.
function buildSyntheticFrame(points: Array<[number, number, number]>): ArrayBuffer {
  return buildSyntheticGrid(points, points.length, 1);
}

function buildSyntheticGrid(
  points: Array<[number, number, number]>,
  width: number,
  height: number,
  rowPaddingBytes = 0,
): ArrayBuffer {
  const pointStep = 16;
  const rowStep = pointStep * width + rowPaddingBytes;
  const data = new Uint8Array(rowStep * height);
  const dv = new DataView(data.buffer);
  for (let i = 0; i < points.length; i++) {
    const row = Math.floor(i / width);
    const col = i % width;
    if (row >= height) break;
    const base = row * rowStep + col * pointStep;
    const p = points[i]!;
    dv.setFloat32(base + 0, p[0], true);
    dv.setFloat32(base + 4, p[1], true);
    dv.setFloat32(base + 8, p[2], true);
    // bytes 12-15: padding (zeros) — matches the K1's 16-byte stride
  }

  // Use a dynamic CDR builder so the test is faithful to the real
  // alignment rules.
  const w = new CdrWriter();
  // header.stamp: int32 sec, uint32 nanosec
  w.writeInt32(0);
  w.writeUint32(0);
  // header.frame_id: string "test_frame" (10 chars + null = 11)
  w.writeString("test_frame");
  w.writeUint32(height);
  w.writeUint32(width);
  // fields[]: 3 entries (x, y, z), all FLOAT32, count=1
  w.writeUint32(3);
  for (const [name, offset] of [
    ["x", 0],
    ["y", 4],
    ["z", 8],
  ] as const) {
    w.writeString(name);
    w.writeUint32(offset);
    w.writeUint8(7); // FLOAT32
    w.writeUint32(1);
  }
  // is_bigendian = false
  w.writeUint8(0);
  // point_step
  w.writeUint32(pointStep);
  w.writeUint32(rowStep);
  // data[]: length-prefixed bytes
  w.writeUint32(data.byteLength);
  w.writeBytes(data);
  // is_dense = true
  w.writeUint8(1);
  return w.finish();
}

describe("decodePointCloud2", () => {
  it("extracts xyz from a 3-point synthetic frame", () => {
    const buf = buildSyntheticFrame([
      [1, 2, 3],
      [4, 5, 6],
      [-7, -8, -9],
    ]);
    const decoded = decodePointCloud2(buf);
    expect(decoded.pointCount).toBe(3);
    expect(Array.from(decoded.positions)).toEqual([
      1, 2, 3, 4, 5, 6, -7, -8, -9,
    ]);
  });

  it("filters NaN points and reports the surviving count", () => {
    const buf = buildSyntheticFrame([
      [1, 2, 3],
      [Number.NaN, 0, 0],
      [4, 5, 6],
      [0, Number.NaN, 0],
    ]);
    const decoded = decodePointCloud2(buf);
    expect(decoded.pointCount).toBe(2);
    expect(Array.from(decoded.positions)).toEqual([1, 2, 3, 4, 5, 6]);
  });

  it("rejects a buffer too short for a CDR header", () => {
    expect(() => decodePointCloud2(new ArrayBuffer(2))).toThrow(/too short/);
  });

  it("rejects an unknown CDR encapsulation byte", () => {
    const buf = new ArrayBuffer(16);
    new DataView(buf).setUint8(1, 0x42); // bogus repr_id
    expect(() => decodePointCloud2(buf)).toThrow(/repr_id/);
  });

  it("decodes an empty cloud (zero points) without throwing", () => {
    const buf = buildSyntheticFrame([]);
    const decoded = decodePointCloud2(buf);
    expect(decoded.pointCount).toBe(0);
    expect(decoded.positions.length).toBe(0);
  });

  it("infers pinhole projection from organized optical samples", () => {
    const width = 8;
    const height = 6;
    const fx = 7.5;
    const fy = 8.25;
    const cx = 3.5;
    const cy = 2.5;
    const z = 2;
    const points: Array<[number, number, number]> = [];
    for (let v = 0; v < height; v++) {
      for (let u = 0; u < width; u++) {
        points.push([((u - cx) * z) / fx, ((v - cy) * z) / fy, z]);
      }
    }

    const decoded = decodePointCloud2(
      buildSyntheticGrid(points, width, height, 8),
    );

    expect(decoded.pointCount).toBe(width * height);
    expect(decoded.width).toBe(width);
    expect(decoded.height).toBe(height);
    expect(decoded.projection).toBeDefined();
    expect(decoded.projection?.fx).toBeCloseTo(fx, 4);
    expect(decoded.projection?.fy).toBeCloseTo(fy, 4);
    expect(decoded.projection?.cx).toBeCloseTo(cx, 4);
    expect(decoded.projection?.cy).toBeCloseTo(cy, 4);
    expect(decoded.projection?.sampleCount).toBe(width * height);
  });
});

// Minimal CDR writer matching the reader's body-relative alignment
// rules. Used only in this test file.
class CdrWriter {
  private bytes: number[] = [];
  private readonly bodyStart = 4;

  constructor() {
    // Encapsulation header: PL=0 + repr_id=0x01 (CDR LE) + 0x00 0x00.
    this.bytes.push(0x00, 0x01, 0x00, 0x00);
  }

  writeInt32(v: number): void {
    this.align(4);
    const buf = new ArrayBuffer(4);
    new DataView(buf).setInt32(0, v, true);
    this.pushBuf(buf);
  }

  writeUint32(v: number): void {
    this.align(4);
    const buf = new ArrayBuffer(4);
    new DataView(buf).setUint32(0, v, true);
    this.pushBuf(buf);
  }

  writeUint8(v: number): void {
    this.bytes.push(v & 0xff);
  }

  writeString(s: string): void {
    const enc = new TextEncoder().encode(s);
    this.writeUint32(enc.byteLength + 1); // includes null
    for (const b of enc) this.bytes.push(b);
    this.bytes.push(0);
  }

  writeBytes(b: Uint8Array): void {
    for (const x of b) this.bytes.push(x);
  }

  finish(): ArrayBuffer {
    return new Uint8Array(this.bytes).buffer;
  }

  private align(n: number): void {
    const bodyOffset = this.bytes.length - this.bodyStart;
    const pad = (n - (bodyOffset % n)) % n;
    for (let i = 0; i < pad; i++) this.bytes.push(0);
  }

  private pushBuf(buf: ArrayBuffer): void {
    const u = new Uint8Array(buf);
    for (const b of u) this.bytes.push(b);
  }
}
