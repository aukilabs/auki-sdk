export type JsonObject = Record<string, unknown>;

export type DecodedJsonFrame = {
  value: JsonObject;
  consumed: number;
};

export class FrameError extends Error {
  constructor(
    readonly code:
      | "unexpected_eof"
      | "length_prefix_too_long"
      | "length_overflow"
      | "non_minimal_length"
      | "body_too_large"
      | "truncated_body"
      | "invalid_utf8"
      | "invalid_json"
      | "body_not_object",
    message: string,
  ) {
    super(message);
    this.name = "FrameError";
  }
}

const MAX_LEB128_U64_BYTES = 10;
const MAX_U64 = (1n << 64n) - 1n;

export function encodeLength(value: bigint | number): Uint8Array {
  let remaining = typeof value === "bigint" ? value : BigInt(value);
  if (remaining < 0n || remaining > MAX_U64) {
    throw new FrameError("length_overflow", "length prefix exceeds u64 range");
  }
  const out: number[] = [];
  do {
    let byte = Number(remaining & 0x7fn);
    remaining >>= 7n;
    if (remaining !== 0n) byte |= 0x80;
    out.push(byte);
  } while (remaining !== 0n);
  return new Uint8Array(out);
}

export function decodeLength(input: Uint8Array, maxBodyLen: bigint | number): [bigint, number] {
  const max = typeof maxBodyLen === "bigint" ? maxBodyLen : BigInt(maxBodyLen);
  let value = 0n;

  for (let index = 0; index < MAX_LEB128_U64_BYTES; index += 1) {
    const byte = input[index];
    if (byte === undefined) {
      throw new FrameError("unexpected_eof", "unexpected eof while reading length prefix");
    }
    const payload = BigInt(byte & 0x7f);
    if (index === MAX_LEB128_U64_BYTES - 1 && payload > 1n) {
      throw new FrameError("length_overflow", "length prefix exceeds u64 range");
    }
    value |= payload << BigInt(index * 7);
    if ((byte & 0x80) === 0) {
      const consumed = index + 1;
      if (encodeLength(value).byteLength !== consumed) {
        throw new FrameError("non_minimal_length", "length prefix is not minimally encoded");
      }
      if (value > max) {
        throw new FrameError("body_too_large", `frame body too large: ${value} bytes`);
      }
      return [value, consumed];
    }
  }

  throw new FrameError("length_prefix_too_long", "length prefix exceeds ten bytes");
}

export function encodeJsonFrame(value: JsonObject, maxBodyLen: number): Uint8Array {
  if (!isPlainObject(value)) {
    throw new FrameError("body_not_object", "frame body is not a json object");
  }
  const body = new TextEncoder().encode(JSON.stringify(value));
  if (body.byteLength > maxBodyLen) {
    throw new FrameError("body_too_large", `frame body too large: ${body.byteLength} bytes`);
  }
  const prefix = encodeLength(body.byteLength);
  const frame = new Uint8Array(prefix.byteLength + body.byteLength);
  frame.set(prefix, 0);
  frame.set(body, prefix.byteLength);
  return frame;
}

export function decodeJsonFrame(input: Uint8Array, maxBodyLen: number): DecodedJsonFrame {
  const [bodyLength, prefixLength] = decodeLength(input, maxBodyLen);
  if (bodyLength > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new FrameError("length_overflow", "length prefix exceeds safe JavaScript range");
  }
  const length = Number(bodyLength);
  const bodyStart = prefixLength;
  const bodyEnd = bodyStart + length;
  if (input.byteLength < bodyEnd) {
    throw new FrameError(
      "truncated_body",
      `truncated frame body: expected ${length} bytes, got ${input.byteLength - bodyStart}`,
    );
  }

  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(input.slice(bodyStart, bodyEnd));
  } catch (_error) {
    throw new FrameError("invalid_utf8", "frame body is not valid utf-8");
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch (error) {
    throw new FrameError("invalid_json", error instanceof Error ? error.message : "invalid JSON");
  }
  if (!isPlainObject(parsed)) {
    throw new FrameError("body_not_object", "frame body is not a json object");
  }
  return { value: parsed, consumed: bodyEnd };
}

function isPlainObject(value: unknown): value is JsonObject {
  return !!value && typeof value === "object" && !Array.isArray(value);
}
