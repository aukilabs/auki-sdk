import { readFile } from "node:fs/promises";
import path from "node:path";
import { beforeAll, describe, expect, it } from "vitest";
import {
  type JsonObject,
  ProtocolWasmError,
  createGetRequest,
  createOfferCatalogRequest,
  createPeerBinding,
  createSubscribeRequest,
  decodeJsonFrame,
  decodeLength,
  encodeJsonFrame,
  encodeLength,
  initializeProtocolWasm,
  parseGetRequest,
  parseGetResponse,
  parseOfferCatalogRequest,
  parseOfferCatalogResponse,
  parseSpatialMessage,
  parseSubscribeEnd,
  parseSubscribeRequest,
  parseSubscribeStartResult,
  validateGetResponseForRequest,
  validateSpatialMessageForOffer,
  validateSubscribeDataMessage,
  validateSubscribeEndForOffer,
  validateSubscribeStartForRequest,
  verifyPeerBinding,
} from "./protocol.js";

describe("auki-protocol-wasm adapter", () => {
  beforeAll(async () => {
    await initializeProtocolWasm({
      module_or_path: await readFile(
        path.resolve(process.cwd(), "../auki-protocol-wasm/pkg-web/auki_protocol_wasm_bg.wasm"),
      ),
    });
  });

  it("matches the Rust v1 JSON frame vectors", async () => {
    const fixture = await fixtureJson("v1_json_frames.json");

    for (const vector of fixture.vectors as JsonObject[]) {
      const value = JSON.parse(vector.body_utf8 as string) as JsonObject;
      const frame = await encodeJsonFrame(value, vector.body_len as number);

      expect(hex(frame)).toBe(vector.frame_hex);
      await expect(decodeLength(frame, vector.body_len as number)).resolves.toEqual({
        value: vector.body_len,
        consumed: String(vector.prefix_hex).length / 2,
      });
      await expect(decodeJsonFrame(frame, vector.body_len as number)).resolves.toEqual({
        value,
        consumed: frame.byteLength,
      });
    }

    await expect(encodeLength(128)).resolves.toEqual(new Uint8Array([0x80, 0x01]));
  });

  it("creates and verifies peer bindings through Rust protocol code", async () => {
    const fixture = await fixtureJson("v1_signed_objects.json");
    const inputs = fixture.inputs as JsonObject;
    const expected = fixture.positive as JsonObject;

    const binding = await createPeerBinding(
      bytesFromHex(inputs.delegate_wallet_seed_hex as string),
      inputs.delegate_peer_id as string,
      inputs.peer_binding_issued_at as string,
      "delegate-peer",
    );

    expect(binding).toEqual((expected.peer_binding as JsonObject).object);
    await expect(verifyPeerBinding(binding, inputs.delegate_peer_id as string)).resolves.toEqual({
      wallet_public_key: binding.wallet_public_key,
      peer_id: inputs.delegate_peer_id,
      issued_at: inputs.peer_binding_issued_at,
      label: "delegate-peer",
    });

    await expect(
      verifyPeerBinding(
        ((fixture.negative as JsonObject).peer_binding_wrong_authenticated_peer as JsonObject)
          .object as JsonObject,
        inputs.other_peer_id as string,
      ),
    ).rejects.toMatchObject({
      name: "ProtocolWasmError",
      failureCode: "identity.peer_id_mismatch",
    } satisfies Partial<ProtocolWasmError>);
  });

  it("creates and parses offer-catalog messages through Rust protocol code", async () => {
    const fixture = await fixtureJson("v1_offer_catalogs.json");
    const inputs = fixture.inputs as JsonObject;
    const positive = fixture.positive as JsonObject;
    const negative = fixture.negative as JsonObject;
    const request = (positive.filtered_request as JsonObject).object as JsonObject;
    const response = (positive.response_with_offer as JsonObject).object as JsonObject;

    await expect(
      createOfferCatalogRequest([inputs.domain_id as string], ["sensor.frame"], true),
    ).resolves.toEqual(request);
    await expect(parseOfferCatalogRequest(request)).resolves.toEqual(request);
    await expect(parseOfferCatalogResponse(response)).resolves.toEqual(response);

    await expect(
      parseOfferCatalogResponse((negative.response_duplicate_offer as JsonObject).object as JsonObject),
    ).rejects.toMatchObject({
      name: "ProtocolWasmError",
      failureCode: "offer.invalid_catalog_response",
    } satisfies Partial<ProtocolWasmError>);
  });

  it("creates and validates Get messages through Rust protocol code", async () => {
    const fixture = await fixtureJson("v1_get.json");
    const inputs = fixture.inputs as JsonObject;
    const positive = fixture.positive as JsonObject;
    const negative = fixture.negative as JsonObject;
    const request = (positive.request as JsonObject).object as JsonObject;
    const response = (positive.success_response as JsonObject).object as JsonObject;
    const message = response.message as JsonObject;

    await expect(
      createGetRequest(
        inputs.domain_id as string,
        inputs.offer_id as string,
        { frame: "latest" },
        [inputs.selected_payload_type as string],
        inputs.max_payload_bytes as number,
      ),
    ).resolves.toEqual(request);
    await expect(parseGetRequest(request)).resolves.toEqual(request);
    await expect(parseGetResponse(response)).resolves.toEqual(response);
    await expect(
      validateGetResponseForRequest(
        request,
        response,
        inputs.selected_payload_type as string,
      ),
    ).resolves.toEqual(message);

    await expect(
      validateGetResponseForRequest(
        request,
        (negative.response_payload_type_mismatch as JsonObject).object as JsonObject,
        inputs.selected_payload_type as string,
      ),
    ).rejects.toMatchObject({
      name: "ProtocolWasmError",
      failureCode: "message.invalid_payload",
    } satisfies Partial<ProtocolWasmError>);
  });

  it("creates and validates Subscribe messages through Rust protocol code", async () => {
    const fixture = await fixtureJson("v1_subscribe.json");
    const inputs = fixture.inputs as JsonObject;
    const positive = fixture.positive as JsonObject;
    const negative = fixture.negative as JsonObject;
    const request = (positive.request as JsonObject).object as JsonObject;
    const accept = (positive.accept_start_result as JsonObject).object as JsonObject;
    const data = (positive.data_message as JsonObject).object as JsonObject;
    const end = (positive.end_message as JsonObject).object as JsonObject;

    await expect(
      createSubscribeRequest(
        inputs.domain_id as string,
        inputs.offer_id as string,
        { frame: "latest", stream: "live" },
        [inputs.selected_payload_type as string],
        inputs.max_message_bytes as number,
      ),
    ).resolves.toEqual(request);
    await expect(parseSubscribeRequest(request)).resolves.toEqual(request);
    await expect(parseSubscribeStartResult(accept)).resolves.toEqual(accept);
    await expect(validateSubscribeStartForRequest(request, accept)).resolves.toEqual({
      accepted: true,
      accept,
    });

    await expect(parseSpatialMessage(data)).resolves.toEqual(data);
    await expect(
      validateSpatialMessageForOffer(
        data,
        inputs.domain_id as string,
        inputs.offer_id as string,
        inputs.selected_payload_type as string,
      ),
    ).resolves.toEqual(data);
    await expect(
      validateSubscribeDataMessage(
        accept,
        data,
        undefined,
        inputs.max_message_bytes as number,
      ),
    ).resolves.toEqual(data);

    await expect(parseSubscribeEnd(end)).resolves.toEqual(end);
    await expect(
      validateSubscribeEndForOffer(end, inputs.domain_id as string, inputs.offer_id as string),
    ).resolves.toEqual(end);

    await expect(
      validateSubscribeDataMessage(
        accept,
        (negative.data_message_payload_type_mismatch as JsonObject).object as JsonObject,
        undefined,
        inputs.max_message_bytes as number,
      ),
    ).rejects.toMatchObject({
      name: "ProtocolWasmError",
      failureCode: "message.invalid_payload",
    } satisfies Partial<ProtocolWasmError>);
  });
});

async function fixtureJson(name: string): Promise<JsonObject> {
  const text = await readFile(path.resolve(process.cwd(), `../auki-protocol/vectors/${name}`), "utf8");
  return JSON.parse(text) as JsonObject;
}

function bytesFromHex(value: string): Uint8Array {
  if (!/^[0-9a-f]+$/u.test(value) || value.length % 2 !== 0) {
    throw new Error("expected lowercase even-length hex");
  }
  return Uint8Array.from(
    Array.from({ length: value.length / 2 }, (_, index) =>
      Number.parseInt(value.slice(index * 2, index * 2 + 2), 16),
    ),
  );
}

function hex(value: Uint8Array): string {
  return Array.from(value, (byte) => byte.toString(16).padStart(2, "0")).join("");
}
