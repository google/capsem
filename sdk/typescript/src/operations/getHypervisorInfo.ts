// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {HypervisorInfo} from "../models/HypervisorInfo.js";
import {HypervisorInfoSchema} from "../validation/HypervisorInfo.js";

export async function getHypervisorInfo(
  transport: Transport,
  options: CallOptions = {},
): Promise<HypervisorInfo> {
  const payload = await transport.request(Method.GET, "/status", {
    signal: options.signal, accept: MediaType.JSON,
  });
  return z.lazy(() => HypervisorInfoSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
