import type { AukiAuthFailureCode } from "./AukiSdkExpo.types";

/** Shared failure code. Auth detail lives on `authCode`, not a second vocabulary. */
export type AukiErrorCode =
  | "cancelled"
  | "closed"
  | "timeout"
  | "auth"
  | "input"
  | "response"
  | "http"
  | "transport"
  | "limit"
  | "callback"
  | "cleanup"
  | "submission_uncertain"
  | "submission_in_progress";

const aliases: Record<string, AukiErrorCode> = {
  invalid_input: "input",
  invalid_response: "response",
  http_status: "http",
  timed_out: "timeout",
  too_large: "limit",
  input: "input",
  response: "response",
  http: "http",
  timeout: "timeout",
  limit: "limit",
  auth: "auth",
  cancelled: "cancelled",
  closed: "closed",
  transport: "transport",
  callback: "callback",
  cleanup: "cleanup",
  submission_uncertain: "submission_uncertain",
  submission_in_progress: "submission_in_progress",
};

const authCodes = new Set<AukiAuthFailureCode>([
  "authentication_required", "configuration", "authorization_denied",
  "persistence", "transient", "cancelled", "closed",
]);

export class AukiError extends Error {
  readonly code: AukiErrorCode;
  readonly status?: number;
  readonly authCode?: AukiAuthFailureCode;
  readonly source?: string;
  readonly retryAfterSeconds?: number;

  constructor(
    code: AukiErrorCode,
    message: string,
    options: {
      cause?: unknown;
      status?: number;
      authCode?: AukiAuthFailureCode;
      source?: string;
      retryAfterSeconds?: number;
    } = {},
  ) {
    super(message);
    this.name = "AukiError";
    this.code = code;
    if (options.cause !== undefined) {
      Object.defineProperty(this, "cause", { value: options.cause });
    }
    if (options.status !== undefined) this.status = options.status;
    if (options.authCode !== undefined) this.authCode = options.authCode;
    if (options.source !== undefined) this.source = options.source;
    if (options.retryAfterSeconds !== undefined) this.retryAfterSeconds = options.retryAfterSeconds;
  }
}

function asCode(value: string): AukiErrorCode | null {
  return aliases[value] ?? null;
}

function asAuthCode(value: string): AukiAuthFailureCode | null {
  return authCodes.has(value as AukiAuthFailureCode) ? value as AukiAuthFailureCode : null;
}

function integer(value: string | undefined): number | undefined {
  if (!value || !Number.isInteger(Number(value))) return undefined;
  return Number(value);
}

export function aukiError(
  code: AukiErrorCode,
  message: string,
  options: ConstructorParameters<typeof AukiError>[2] = {},
): AukiError {
  return new AukiError(code, message, options);
}

function fromPrefixed(raw: string, message: string, cause: Error): AukiError | null {
  const [prefix, kindRaw, statusRaw, third, fourth, fifth] = raw.split(":");
  if (prefix !== "jobs" && prefix !== "fleet" && prefix !== "domain_data") return null;
  const code = kindRaw ? asCode(kindRaw) : null;
  if (!code) return null;
  const status = integer(statusRaw);
  if (prefix === "jobs") {
    const authCode = code === "auth" && third ? asAuthCode(third) ?? undefined : undefined;
    const retry = fifth && /^\d+$/.test(fifth) && Number(fifth) <= 0xffffffff ? Number(fifth) : undefined;
    return aukiError(code, message, {
      cause,
      status,
      authCode,
      source: fourth || undefined,
      retryAfterSeconds: retry,
    });
  }
  const detail = third ? asAuthCode(third) : null;
  return aukiError(code, message, {
    cause,
    status,
    authCode: code === "auth" && detail ? detail : undefined,
  });
}

export function normalizeAukiError(value: unknown, fallback: string): AukiError {
  if (value instanceof AukiError) return value;
  if (!(value instanceof Error)) {
    const text = typeof value === "string" ? value.trim() : "";
    return aukiError("transport", text || fallback, { cause: value });
  }
  const candidate = value as Error & {
    code?: unknown;
    kind?: unknown;
    status?: unknown;
    authCode?: unknown;
    source?: unknown;
    retryAfterSeconds?: unknown;
  };
  if (typeof candidate.code === "string" && candidate.code.includes(":")) {
    const prefixed = fromPrefixed(candidate.code, candidate.message, candidate);
    if (prefixed) return prefixed;
  }
  const fromKind = typeof candidate.kind === "string" ? asCode(candidate.kind) : null;
  const fromCode = typeof candidate.code === "string" ? asCode(candidate.code) : null;
  const status = typeof candidate.status === "number" ? candidate.status : undefined;
  const source = typeof candidate.source === "string" ? candidate.source : undefined;
  const retryAfterSeconds = typeof candidate.retryAfterSeconds === "number"
    ? candidate.retryAfterSeconds
    : undefined;
  if (fromKind) {
    const detail = typeof candidate.code === "string" && !asCode(candidate.code)
      ? asAuthCode(candidate.code)
      : null;
    const authCode = fromKind === "auth"
      ? detail ?? (typeof candidate.authCode === "string" ? asAuthCode(candidate.authCode) ?? undefined : undefined)
      : undefined;
    return aukiError(fromKind, candidate.message, { cause: candidate, status, authCode, source, retryAfterSeconds });
  }
  if (fromCode) {
    return aukiError(fromCode, candidate.message, { cause: candidate, status, source, retryAfterSeconds });
  }
  if (typeof candidate.code === "string") {
    const authCode = asAuthCode(candidate.code);
    if (authCode && authCode !== "cancelled" && authCode !== "closed") {
      return aukiError("auth", candidate.message, { cause: candidate, status, authCode });
    }
  }
  const message = candidate.message.trim();
  return aukiError("transport", message || fallback, { cause: value });
}
