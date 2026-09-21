// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {NetworkInfo} from "../models/NetworkInfo.js";
import {NetworkInfoSchema} from "../validation/NetworkInfo.js";

export async function attachNetworkMember(
  transport: Transport,
  parameters: {
    "id": string;
    "vm_id": string;
  },
  options: CallOptions = {},
): Promise<NetworkInfo> {
  const input = z.object({
  "id": z.string(),
  "vm_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.PUT, "/networks/{id}/members/{vm_id}", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"], "vm_id": input["vm_id"]},
  });
  return z.lazy(() => NetworkInfoSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
