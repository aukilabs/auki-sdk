// Generated TypeScript bindings for the Auki browser control-plane protos:
// - crates/auki-datatypes/proto/join.proto
// - crates/auki-datatypes/proto/info.proto
//
// The libp2p substream framing is a 4-byte big-endian length prefix followed
// by protobuf bytes, matching auki-network's Rust helpers.

export const JOIN_PROTOCOL = "/auki/join/0.0.1";
export const INFO_PROTOCOL = "/auki/info/0.0.1";
export const STREAM_PROTOCOL = "/auki/stream/0.1.0";

export const MAX_CONTROL_FRAME_BYTES = 1024 * 1024;

export type ProtocolMessage<T> = {
  encode(message: T): Uint8Array;
  decode(bytes: Uint8Array): T;
};

export type ProtocolStream = AsyncIterable<Uint8Array | { subarray(): Uint8Array }> & {
  send(data: Uint8Array): boolean;
  close(options?: unknown): Promise<void>;
  onDrain?(options?: unknown): Promise<void>;
};

export type JoinRequest = {
  multiaddrs: string[];
};

export const JoinRequest: ProtocolMessage<JoinRequest> = {
  encode(message) {
    const writer = new ProtoWriter();
    for (const addr of message.multiaddrs) {
      writer.string(1, addr);
    }
    return writer.finish();
  },
  decode(bytes) {
    const reader = new ProtoReader(bytes);
    const message: JoinRequest = { multiaddrs: [] };
    while (!reader.done()) {
      const { field, wireType } = reader.tag();
      if (field === 1 && wireType === 2) {
        message.multiaddrs.push(reader.string());
      } else {
        reader.skip(wireType);
      }
    }
    return message;
  },
};

export type JoinResponse =
  | {
      kind: {
        case: "accept";
        value: {
          membershipJson: string;
          successorToken: Uint8Array<ArrayBufferLike>;
        };
      };
    }
  | {
      kind: {
        case: "reject";
        value: {
          reason: string;
        };
      };
    }
  | {
      kind?: undefined;
    };

export const JoinResponse: ProtocolMessage<JoinResponse> = {
  encode(message) {
    const writer = new ProtoWriter();
    if (message.kind?.case === "accept") {
      const inner = new ProtoWriter();
      inner.string(1, message.kind.value.membershipJson);
      inner.bytes(2, message.kind.value.successorToken);
      writer.bytes(1, inner.finish());
    } else if (message.kind?.case === "reject") {
      const inner = new ProtoWriter();
      inner.string(1, message.kind.value.reason);
      writer.bytes(2, inner.finish());
    }
    return writer.finish();
  },
  decode(bytes) {
    const reader = new ProtoReader(bytes);
    const message: JoinResponse = {};
    while (!reader.done()) {
      const { field, wireType } = reader.tag();
      if (field === 1 && wireType === 2) {
        const accept = new ProtoReader(reader.bytes());
        let membershipJson = "";
        let successorToken: Uint8Array<ArrayBufferLike> = new Uint8Array();
        while (!accept.done()) {
          const tag = accept.tag();
          if (tag.field === 1 && tag.wireType === 2) {
            membershipJson = accept.string();
          } else if (tag.field === 2 && tag.wireType === 2) {
            successorToken = accept.bytes();
          } else {
            accept.skip(tag.wireType);
          }
        }
        return { kind: { case: "accept", value: { membershipJson, successorToken } } };
      }
      if (field === 2 && wireType === 2) {
        const reject = new ProtoReader(reader.bytes());
        let reason = "";
        while (!reject.done()) {
          const tag = reject.tag();
          if (tag.field === 1 && tag.wireType === 2) {
            reason = reject.string();
          } else {
            reject.skip(tag.wireType);
          }
        }
        return { kind: { case: "reject", value: { reason } } };
      }
      reader.skip(wireType);
    }
    return message;
  },
};

export type InfoRequest = Record<string, never>;

export const InfoRequest: ProtocolMessage<InfoRequest> = {
  encode() {
    return new Uint8Array();
  },
  decode() {
    return {};
  },
};

export type InfoResponse = {
  participantInfoJson: string;
};

export const InfoResponse: ProtocolMessage<InfoResponse> = {
  encode(message) {
    const writer = new ProtoWriter();
    writer.string(1, message.participantInfoJson);
    return writer.finish();
  },
  decode(bytes) {
    const reader = new ProtoReader(bytes);
    const message: InfoResponse = { participantInfoJson: "" };
    while (!reader.done()) {
      const { field, wireType } = reader.tag();
      if (field === 1 && wireType === 2) {
        message.participantInfoJson = reader.string();
      } else {
        reader.skip(wireType);
      }
    }
    return message;
  },
};

export type StreamRequest = {
  sensorId: string;
};

export type StreamManifest = {
  sensorId: string;
  sensorHash: string;
  clockId: string;
  clockHash: string;
  frameId: string;
  frameHash: string;
};

export type StreamEntry = {
  timestampNs: number;
  seq: number;
  payload: Uint8Array<ArrayBufferLike>;
};

export type StreamMessage =
  | { variant: { case: "request"; value: StreamRequest } }
  | { variant: { case: "accept"; value: StreamManifest } }
  | { variant: { case: "decline"; value: { reason: string } } }
  | { variant: { case: "entry"; value: StreamEntry } }
  | { variant: { case: "endOfStream"; value: { reason: string } } }
  | { variant?: undefined };

export const StreamMessage: ProtocolMessage<StreamMessage> = {
  encode(message) {
    const writer = new ProtoWriter();
    switch (message.variant?.case) {
      case "request": {
        const inner = new ProtoWriter();
        inner.string(1, message.variant.value.sensorId);
        writer.bytes(1, inner.finish());
        break;
      }
      case "accept": {
        const inner = new ProtoWriter();
        inner.string(1, message.variant.value.sensorId);
        inner.string(2, message.variant.value.sensorHash);
        inner.string(3, message.variant.value.clockId);
        inner.string(4, message.variant.value.clockHash);
        inner.string(5, message.variant.value.frameId);
        inner.string(6, message.variant.value.frameHash);
        writer.bytes(2, inner.finish());
        break;
      }
      case "decline": {
        const other = new ProtoWriter();
        other.string(1, message.variant.value.reason);
        const reason = new ProtoWriter();
        reason.bytes(4, other.finish());
        writer.bytes(3, reason.finish());
        break;
      }
      case "entry": {
        const inner = new ProtoWriter();
        inner.uint64(1, message.variant.value.timestampNs);
        inner.uint64(2, message.variant.value.seq);
        inner.bytes(3, message.variant.value.payload);
        writer.bytes(4, inner.finish());
        break;
      }
      case "endOfStream": {
        const error = new ProtoWriter();
        error.string(1, message.variant.value.reason);
        const reason = new ProtoWriter();
        reason.bytes(4, error.finish());
        writer.bytes(5, reason.finish());
        break;
      }
    }
    return writer.finish();
  },
  decode(bytes) {
    const reader = new ProtoReader(bytes);
    while (!reader.done()) {
      const { field, wireType } = reader.tag();
      if (wireType !== 2) {
        reader.skip(wireType);
        continue;
      }
      if (field === 1) {
        const inner = new ProtoReader(reader.bytes());
        let sensorId = "";
        while (!inner.done()) {
          const tag = inner.tag();
          if (tag.field === 1 && tag.wireType === 2) sensorId = inner.string();
          else inner.skip(tag.wireType);
        }
        return { variant: { case: "request", value: { sensorId } } };
      }
      if (field === 2) {
        const inner = new ProtoReader(reader.bytes());
        const value: StreamManifest = {
          sensorId: "",
          sensorHash: "",
          clockId: "",
          clockHash: "",
          frameId: "",
          frameHash: "",
        };
        while (!inner.done()) {
          const tag = inner.tag();
          if (tag.field === 1 && tag.wireType === 2) value.sensorId = inner.string();
          else if (tag.field === 2 && tag.wireType === 2) value.sensorHash = inner.string();
          else if (tag.field === 3 && tag.wireType === 2) value.clockId = inner.string();
          else if (tag.field === 4 && tag.wireType === 2) value.clockHash = inner.string();
          else if (tag.field === 5 && tag.wireType === 2) value.frameId = inner.string();
          else if (tag.field === 6 && tag.wireType === 2) value.frameHash = inner.string();
          else inner.skip(tag.wireType);
        }
        return { variant: { case: "accept", value } };
      }
      if (field === 3) {
        reader.bytes();
        return { variant: { case: "decline", value: { reason: "declined" } } };
      }
      if (field === 4) {
        const inner = new ProtoReader(reader.bytes());
        const value: StreamEntry = { timestampNs: 0, seq: 0, payload: new Uint8Array() };
        while (!inner.done()) {
          const tag = inner.tag();
          if (tag.field === 1 && tag.wireType === 0) value.timestampNs = inner.varint();
          else if (tag.field === 2 && tag.wireType === 0) value.seq = inner.varint();
          else if (tag.field === 3 && tag.wireType === 2) value.payload = inner.bytes();
          else inner.skip(tag.wireType);
        }
        return { variant: { case: "entry", value } };
      }
      if (field === 5) {
        reader.bytes();
        return { variant: { case: "endOfStream", value: { reason: "ended" } } };
      }
      reader.skip(wireType);
    }
    return {};
  },
};

export type AudioData = {
  data: Uint8Array<ArrayBufferLike>;
};

export const AudioData: ProtocolMessage<AudioData> = {
  encode(message) {
    const writer = new ProtoWriter();
    writer.bytes(1, message.data);
    return writer.finish();
  },
  decode(bytes) {
    const reader = new ProtoReader(bytes);
    const message: AudioData = { data: new Uint8Array() };
    while (!reader.done()) {
      const { field, wireType } = reader.tag();
      if (field === 1 && wireType === 2) message.data = reader.bytes();
      else reader.skip(wireType);
    }
    return message;
  },
};

export function encodeFrame(payload: Uint8Array): Uint8Array {
  if (payload.byteLength > MAX_CONTROL_FRAME_BYTES) {
    throw new Error(
      `control frame too large: ${payload.byteLength} bytes (max ${MAX_CONTROL_FRAME_BYTES})`,
    );
  }
  const frame = new Uint8Array(payload.byteLength + 4);
  new DataView(frame.buffer, frame.byteOffset, frame.byteLength).setUint32(0, payload.byteLength);
  frame.set(payload, 4);
  return frame;
}

export function decodeFrame(frame: Uint8Array): Uint8Array {
  if (frame.byteLength < 4) {
    throw new Error("control frame missing 4-byte length prefix");
  }
  const view = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
  const length = view.getUint32(0);
  if (length > MAX_CONTROL_FRAME_BYTES) {
    throw new Error(`control frame too large: ${length} bytes (max ${MAX_CONTROL_FRAME_BYTES})`);
  }
  if (frame.byteLength - 4 < length) {
    throw new Error(`control frame truncated: expected ${length} bytes`);
  }
  return frame.slice(4, 4 + length);
}

export async function writeFrame<T>(
  stream: ProtocolStream,
  codec: ProtocolMessage<T>,
  message: T,
): Promise<void> {
  if (!stream.send(encodeFrame(codec.encode(message))) && stream.onDrain) {
    await stream.onDrain();
  }
}

export async function readFrame<T>(
  stream: ProtocolStream,
  codec: ProtocolMessage<T>,
): Promise<T> {
  const reader = new FrameStreamReader(stream);
  return codec.decode(await reader.readFrame());
}

class FrameStreamReader {
  private readonly iterator: AsyncIterator<Uint8Array | { subarray(): Uint8Array }>;
  private buffer = new Uint8Array();

  constructor(stream: AsyncIterable<Uint8Array | { subarray(): Uint8Array }>) {
    this.iterator = stream[Symbol.asyncIterator]();
  }

  async readFrame(): Promise<Uint8Array> {
    await this.ensure(4);
    const length = new DataView(this.buffer.buffer, this.buffer.byteOffset, 4).getUint32(0);
    if (length > MAX_CONTROL_FRAME_BYTES) {
      throw new Error(`control frame too large: ${length} bytes (max ${MAX_CONTROL_FRAME_BYTES})`);
    }
    await this.ensure(4 + length);
    const payload = this.buffer.slice(4, 4 + length);
    this.buffer = this.buffer.slice(4 + length);
    return payload;
  }

  private async ensure(length: number): Promise<void> {
    while (this.buffer.byteLength < length) {
      const next = await this.iterator.next();
      if (next.done) {
        throw new Error("control stream closed before a complete frame arrived");
      }
      const chunk = normalizeChunk(next.value);
      const merged = new Uint8Array(this.buffer.byteLength + chunk.byteLength);
      merged.set(this.buffer);
      merged.set(chunk, this.buffer.byteLength);
      this.buffer = merged;
    }
  }
}

function normalizeChunk(chunk: Uint8Array | { subarray(): Uint8Array }): Uint8Array {
  return chunk instanceof Uint8Array ? chunk : chunk.subarray();
}

class ProtoWriter {
  private readonly chunks: number[] = [];

  bool(field: number, value: boolean): void {
    this.tag(field, 0);
    this.varint(value ? 1 : 0);
  }

  string(field: number, value: string): void {
    this.bytes(field, new TextEncoder().encode(value));
  }

  bytes(field: number, value: Uint8Array): void {
    this.tag(field, 2);
    this.varint(value.byteLength);
    for (const byte of value) this.chunks.push(byte);
  }

  finish(): Uint8Array {
    return new Uint8Array(this.chunks);
  }

  private tag(field: number, wireType: number): void {
    this.varint((field << 3) | wireType);
  }

  uint64(field: number, value: number): void {
    this.tag(field, 0);
    this.varint(value);
  }

  private varint(value: number): void {
    let next = value >>> 0;
    while (next >= 0x80) {
      this.chunks.push((next & 0x7f) | 0x80);
      next >>>= 7;
    }
    this.chunks.push(next);
  }
}

class ProtoReader {
  private offset = 0;

  constructor(private readonly bytes_: Uint8Array) {}

  done(): boolean {
    return this.offset >= this.bytes_.byteLength;
  }

  tag(): { field: number; wireType: number } {
    const tag = this.varint();
    return { field: tag >>> 3, wireType: tag & 0x07 };
  }

  varint(): number {
    let result = 0;
    let shift = 0;
    while (shift < 35) {
      if (this.offset >= this.bytes_.byteLength) throw new Error("protobuf varint truncated");
      const byte = this.bytes_[this.offset++];
      result |= (byte & 0x7f) << shift;
      if ((byte & 0x80) === 0) return result >>> 0;
      shift += 7;
    }
    throw new Error("protobuf varint too long");
  }

  string(): string {
    return new TextDecoder().decode(this.bytes());
  }

  bytes(): Uint8Array {
    const length = this.varint();
    const end = this.offset + length;
    if (end > this.bytes_.byteLength) throw new Error("protobuf bytes field truncated");
    const value = this.bytes_.slice(this.offset, end);
    this.offset = end;
    return value;
  }

  skip(wireType: number): void {
    if (wireType === 0) {
      this.varint();
      return;
    }
    if (wireType === 2) {
      this.bytes();
      return;
    }
    if (wireType === 5) {
      this.offset += 4;
      return;
    }
    if (wireType === 1) {
      this.offset += 8;
      return;
    }
    throw new Error(`unsupported protobuf wire type ${wireType}`);
  }
}
