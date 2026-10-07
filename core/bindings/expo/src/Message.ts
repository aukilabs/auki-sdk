import module from "./AukiSdkExpoModule";
import type {
  AukiExactTarget,
  AukiMessageChannelInfo,
  AukiMessageEventInfo,
  AukiMessageSenderInfo,
} from "./AukiSdkExpo.types";

/** Protocol ID to advertise in a peer card when this peer receives messages. */
export const MESSAGE_PROTOCOL_ID = "/auki/auth/1/message/0.1.0";

/** One received message with its payload decoded to bytes. */
export type AukiMessage = {
  channel: AukiMessageChannelInfo;
  sender: AukiMessageSenderInfo;
  messageType: string;
  /** Signed 64-bit nanoseconds, preserved as text. */
  timestampNs: string;
  payload: Uint8Array;
};

function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

function base64ToBytes(value: string): Uint8Array {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

function decodeEvent(encoded: string): AukiMessage {
  const event = JSON.parse(encoded) as AukiMessageEventInfo;
  return {
    channel: event.channel,
    sender: event.sender,
    messageType: event.messageType,
    timestampNs: event.timestampNs,
    payload: base64ToBytes(event.payloadBase64),
  };
}

/** One declared channel. Read it serially with `next()` or `for await`. */
export class AukiMessageReceiver implements AsyncIterable<AukiMessage> {
  private pending = false;
  private closed = false;

  /** @internal Use AukiMessageEndpoint.declare(). */
  constructor(
    private readonly handle: string,
    readonly channel: AukiMessageChannelInfo,
  ) {}

  /** Next message, or null once the receiver is closed or undeclared. */
  async next(): Promise<AukiMessage | null> {
    if (this.closed) return null;
    if (this.pending) throw new Error("AukiMessageReceiver.next() is already pending");
    this.pending = true;
    try {
      const encoded = await module.messageNext(this.handle);
      return encoded === null ? null : decodeEvent(encoded);
    } finally {
      this.pending = false;
    }
  }

  async *[Symbol.asyncIterator](): AsyncIterator<AukiMessage> {
    for (;;) {
      const message = await this.next();
      if (message === null) return;
      yield message;
    }
  }

  /** Undeclare the channel. A pending `next()` resolves with null. */
  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    await module.messageReceiverClose(this.handle);
  }
}

/** Inbound Message v1 service mounted on one running peer. */
export class AukiMessageEndpoint {
  private closed = false;

  /** @internal Use AukiMessageEndpoint.mount(). */
  constructor(private readonly handle: string) {}

  static async mount(peerHandle: string): Promise<AukiMessageEndpoint> {
    return new AukiMessageEndpoint(await module.messageMount(peerHandle));
  }

  /** Declare one receiver-owned channel; `capacity` bounds its native queue (1..=65536). */
  async declare(channel: AukiMessageChannelInfo, capacity = 256): Promise<AukiMessageReceiver> {
    if (this.closed) throw new Error("AukiMessageEndpoint is closed");
    const handle = await module.messageDeclare(this.handle, JSON.stringify(channel), capacity);
    return new AukiMessageReceiver(handle, channel);
  }

  /** Stop declarations and wait for every admitted handler. */
  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    await module.messageEndpointClose(this.handle);
  }
}

/** Sending side of one remote channel. Each `send` waits for the receiver's acknowledgement. */
export class AukiMessageSender {
  private closed = false;

  /** @internal Use AukiMessageSender.open(). */
  constructor(private readonly handle: string) {}

  static async open(
    peerHandle: string,
    target: AukiExactTarget,
    channel: AukiMessageChannelInfo,
  ): Promise<AukiMessageSender> {
    return new AukiMessageSender(
      await module.messageOpenExact(peerHandle, target, JSON.stringify(channel)),
    );
  }

  async send(messageType: string, timestampNs: bigint | string, payload: Uint8Array): Promise<void> {
    if (this.closed) throw new Error("AukiMessageSender is closed");
    await module.messageSend(this.handle, messageType, String(timestampNs), bytesToBase64(payload));
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    await module.messageClose(this.handle);
  }
}
