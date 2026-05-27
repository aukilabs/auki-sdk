// Minimal CDR parser scoped to ROS2 `sensor_msgs/PointCloud2`.
//
// The Booster-side producer (per Dagaz D2) forwards `PointCloud2`
// message bytes verbatim. The SDK's `auki.point_cloud.Data { data }`
// carries those bytes through libp2p; Park's HTTP route serves them
// raw at `/api/streams/<peer>/<sensor>/latest.cdr`. The browser
// parses CDR here.
//
// We extract just what the renderer needs: an interleaved
// `Float32Array` of xyz positions, NaN-filtered. Other fields
// (rgb / intensity / curvature / etc.) can be added later by
// extending `decodePointCloud2`.
//
// ## CDR primer
//
// ROS2 uses OMG CDR (XCDR1) by default. Layout:
//
// - 4-byte encapsulation header: `[0x00, repr_id, 0x00, 0x00]`.
//   `repr_id == 0x01` means CDR LE, `0x00` means CDR BE. ROS2 ships
//   LE on every platform we've ever seen, but we honour the flag.
// - Message body starts at byte 4. **All primitive alignments are
//   relative to the body start**, not the buffer start.
// - Primitives align to their size (uint8/bool=1, uint16=2,
//   uint32/float32/int32=4, uint64/float64=8). Padding bytes are
//   inserted before each primitive to satisfy the alignment.
// - Strings: `uint32` length (including the null terminator), then
//   bytes (with the null terminator). Aligns to 4 for the length.
// - Sequences (variable-length arrays): `uint32` count, then `count`
//   elements. Each element follows its own primitive alignment.
//
// ## `PointCloud2` schema
//
// ```
// std_msgs/Header header
//   builtin_interfaces/Time stamp { int32 sec; uint32 nanosec; }
//   string frame_id
// uint32 height
// uint32 width
// sensor_msgs/PointField[] fields
//   string name
//   uint32 offset
//   uint8  datatype
//   uint32 count
// bool is_bigendian
// uint32 point_step
// uint32 row_step
// uint8[] data
// bool is_dense
// ```
//
// `data[]` is a row-major byte buffer. Each row is `row_step` bytes,
// and the first `width * point_step` bytes in that row hold the points.
// Each point's xyz live at the offsets the producer declared in the
// `fields[]` array.

/** ROS2 `sensor_msgs/msg/PointField` datatype enum. */
const FLOAT32_DATATYPE = 7;
// Other datatypes (INT8=1, UINT8=2, INT16=3, UINT16=4, INT32=5,
// UINT32=6, FLOAT64=8) aren't supported here — the K1's StereoNetNode
// produces FLOAT32 xyz and that's all the renderer needs today.

export type DecodedPointCloud = {
  /** ROS Header.frame_id from the CDR payload. */
  frameId: string;
  /** Organized cloud height from `PointCloud2.height`. */
  height: number;
  /** Organized cloud width from `PointCloud2.width`. */
  width: number;
  /** Interleaved xyz, length = pointCount * 3. */
  positions: Float32Array;
  /** Number of valid (non-NaN) points after filtering. */
  pointCount: number;
  /** Pinhole projection inferred from organized xyz samples, when possible. */
  projection?: InferredPinholeProjection;
};

export type InferredPinholeProjection = {
  width: number;
  height: number;
  fx: number;
  fy: number;
  cx: number;
  cy: number;
  sampleCount: number;
};

/** Parse a CDR-encoded ROS2 `sensor_msgs/PointCloud2` message into an
 * interleaved xyz `Float32Array`. NaN points are filtered (per the
 * `is_dense` contract — non-dense clouds may carry NaN sentinels for
 * invalid measurements). */
export function decodePointCloud2(buffer: ArrayBuffer): DecodedPointCloud {
  if (buffer.byteLength < 4) {
    throw new Error(`CDR buffer too short: ${buffer.byteLength} bytes`);
  }
  const view = new DataView(buffer);
  const reprId = view.getUint8(1);
  if (reprId !== 0x00 && reprId !== 0x01) {
    throw new Error(`unknown CDR encapsulation repr_id 0x${reprId.toString(16)}`);
  }
  const messageLE = reprId === 0x01;

  const r = new CdrReader(view, messageLE);
  // Header.stamp { int32 sec; uint32 nanosec }
  r.readInt32();
  r.readUint32();
  const frameId = r.readString();
  // height, width
  const height = r.readUint32();
  const width = r.readUint32();
  // fields[]
  const fieldsLen = r.readUint32();
  let xOffset = -1;
  let yOffset = -1;
  let zOffset = -1;
  for (let i = 0; i < fieldsLen; i++) {
    const name = r.readString();
    const offset = r.readUint32();
    const datatype = r.readUint8();
    const count = r.readUint32();
    if (count !== 1) continue; // not a scalar — skip
    if (datatype !== FLOAT32_DATATYPE) continue;
    if (name === "x") xOffset = offset;
    else if (name === "y") yOffset = offset;
    else if (name === "z") zOffset = offset;
  }
  if (xOffset < 0 || yOffset < 0 || zOffset < 0) {
    throw new Error(
      `PointCloud2 missing FLOAT32 x/y/z fields (got x=${xOffset}, y=${yOffset}, z=${zOffset})`,
    );
  }
  // is_bigendian governs the byte order INSIDE each point payload —
  // independent of the CDR-level encapsulation that gated the schema
  // reads above. ROS2 publishers on every platform we've seen write
  // LE here; honour the flag anyway.
  const dataIsBigEndian = r.readBool();
  const pointStep = r.readUint32();
  const rowStep = r.readUint32();
  const dataLen = r.readUint32();
  // The data sequence body starts at the reader's current absolute
  // offset and runs for `dataLen` bytes. We read points directly
  // out of the underlying buffer rather than copying them through
  // the reader.
  const dataAbsStart = r.absoluteOffset();
  if (dataAbsStart + dataLen > buffer.byteLength) {
    throw new Error(
      `PointCloud2 data overruns buffer (start=${dataAbsStart}, len=${dataLen}, buffer=${buffer.byteLength})`,
    );
  }
  if (pointStep === 0) {
    throw new Error("PointCloud2 point_step is 0");
  }

  const pointsLE = !dataIsBigEndian;
  const maxPossiblePoints = Math.floor(dataLen / pointStep);
  const gridPointCount = width > 0 && height > 0 ? width * height : maxPossiblePoints;
  const organizedLayout = width > 0 && height > 0 && rowStep >= width * pointStep;
  const positions = new Float32Array(
    (organizedLayout ? gridPointCount : maxPossiblePoints) * 3,
  );
  const projectionFit = new ProjectionFit(width, height);
  let writeIdx = 0;
  const maxFieldOffset = Math.max(xOffset, yOffset, zOffset) + 4;

  const pushPoint = (u: number, v: number, base: number) => {
    if (base + maxFieldOffset > dataAbsStart + dataLen) return;
    const x = view.getFloat32(base + xOffset, pointsLE);
    const y = view.getFloat32(base + yOffset, pointsLE);
    const z = view.getFloat32(base + zOffset, pointsLE);
    if (Number.isNaN(x) || Number.isNaN(y) || Number.isNaN(z)) return;
    projectionFit.add(u, v, x, y, z);
    positions[writeIdx++] = x;
    positions[writeIdx++] = y;
    positions[writeIdx++] = z;
  };

  if (organizedLayout) {
    for (let v = 0; v < height; v++) {
      const rowBase = dataAbsStart + v * rowStep;
      if (rowBase >= dataAbsStart + dataLen) break;
      for (let u = 0; u < width; u++) {
        pushPoint(u, v, rowBase + u * pointStep);
      }
    }
  } else {
    for (let i = 0; i < maxPossiblePoints; i++) {
      const fallbackWidth = Math.max(1, width);
      pushPoint(
        i % fallbackWidth,
        Math.floor(i / fallbackWidth),
        dataAbsStart + i * pointStep,
      );
    }
  }
  const pointCount = writeIdx / 3;
  return {
    frameId,
    height,
    width,
    positions: positions.subarray(0, writeIdx),
    pointCount,
    projection: projectionFit.finish(),
  };
}

class ProjectionFit {
  private xN = 0;
  private xSum = 0;
  private uSum = 0;
  private xxSum = 0;
  private xuSum = 0;
  private yN = 0;
  private ySum = 0;
  private vSum = 0;
  private yySum = 0;
  private yvSum = 0;

  constructor(
    private readonly width: number,
    private readonly height: number,
  ) {}

  add(u: number, v: number, x: number, y: number, z: number): void {
    if (this.width <= 1 || this.height <= 1 || !Number.isFinite(z) || z <= 1e-6) {
      return;
    }
    if (u < 0 || u >= this.width || v < 0 || v >= this.height) return;
    const xn = x / z;
    const yn = y / z;
    if (!Number.isFinite(xn) || !Number.isFinite(yn)) return;
    this.xN += 1;
    this.xSum += xn;
    this.uSum += u;
    this.xxSum += xn * xn;
    this.xuSum += xn * u;
    this.yN += 1;
    this.ySum += yn;
    this.vSum += v;
    this.yySum += yn * yn;
    this.yvSum += yn * v;
  }

  finish(): InferredPinholeProjection | undefined {
    const xFit = linearFit(this.xN, this.xSum, this.uSum, this.xxSum, this.xuSum);
    const yFit = linearFit(this.yN, this.ySum, this.vSum, this.yySum, this.yvSum);
    if (!xFit || !yFit) return undefined;
    const projection = {
      width: this.width,
      height: this.height,
      fx: xFit.slope,
      fy: yFit.slope,
      cx: xFit.intercept,
      cy: yFit.intercept,
      sampleCount: Math.min(this.xN, this.yN),
    };
    if (
      projection.fx <= 0 ||
      projection.fy <= 0 ||
      projection.sampleCount < 32 ||
      !Number.isFinite(projection.cx) ||
      !Number.isFinite(projection.cy) ||
      projection.cx < -this.width ||
      projection.cx > this.width * 2 ||
      projection.cy < -this.height ||
      projection.cy > this.height * 2 ||
      !isPlausibleFov(this.width, projection.fx) ||
      !isPlausibleFov(this.height, projection.fy)
    ) {
      return undefined;
    }
    return projection;
  }
}

function linearFit(
  n: number,
  xSum: number,
  ySum: number,
  xxSum: number,
  xySum: number,
): { slope: number; intercept: number } | undefined {
  const denom = n * xxSum - xSum * xSum;
  if (n < 2 || Math.abs(denom) < 1e-9) return undefined;
  const slope = (n * xySum - xSum * ySum) / denom;
  const intercept = (ySum - slope * xSum) / n;
  if (!Number.isFinite(slope) || !Number.isFinite(intercept)) return undefined;
  return { slope, intercept };
}

function isPlausibleFov(spanPx: number, focalPx: number): boolean {
  const fov = 2 * Math.atan(spanPx / (2 * focalPx));
  return fov > MIN_FOV_RAD && fov < MAX_FOV_RAD;
}

const MIN_FOV_RAD = Math.PI / 60;
const MAX_FOV_RAD = (175 * Math.PI) / 180;

/** CDR reader with body-relative alignment. */
class CdrReader {
  private offset = 4; // body starts after the 4-byte encapsulation header
  private readonly bodyStart = 4;

  constructor(
    private readonly view: DataView,
    private readonly littleEndian: boolean,
  ) {}

  /** Absolute offset into the underlying buffer (for byte-array reads
   * that don't go through the reader's own primitive accessors). */
  absoluteOffset(): number {
    return this.offset;
  }

  readInt32(): number {
    this.align(4);
    const v = this.view.getInt32(this.offset, this.littleEndian);
    this.offset += 4;
    return v;
  }

  readUint32(): number {
    this.align(4);
    const v = this.view.getUint32(this.offset, this.littleEndian);
    this.offset += 4;
    return v;
  }

  readUint8(): number {
    // 1-byte alignment is always satisfied
    const v = this.view.getUint8(this.offset);
    this.offset += 1;
    return v;
  }

  readBool(): boolean {
    return this.readUint8() !== 0;
  }

  readString(): string {
    const len = this.readUint32(); // includes null terminator
    if (len === 0) return "";
    // Bytes (length includes the trailing null)
    const bytes = new Uint8Array(this.view.buffer, this.view.byteOffset + this.offset, len - 1);
    const s = new TextDecoder("utf-8").decode(bytes);
    this.offset += len;
    return s;
  }

  /** Align the reader's body-relative position to a multiple of `n`,
   * inserting padding bytes as needed. CDR alignment is computed from
   * the start of the message body (byte 4 of the buffer), not the
   * buffer origin. */
  private align(n: number): void {
    const bodyOffset = this.offset - this.bodyStart;
    const pad = (n - (bodyOffset % n)) % n;
    this.offset += pad;
  }
}
