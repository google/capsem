// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {AssetStatus} from "../models/AssetStatus.js";
import {AssetStatusSchema} from "../validation/AssetStatus.js";

export async function getAssetStatus(
  transport: Transport,
  options: CallOptions = {},
): Promise<AssetStatus> {
  const payload = await transport.request(Method.GET, "/assets/status", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
  });
  return z.lazy(() => AssetStatusSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
