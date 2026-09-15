// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {NetworkInfo} from "../models/NetworkInfo.js";
import {NetworkInfoSchema} from "../validation/NetworkInfo.js";

export async function detachNetworkMember(
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
  const payload = await transport.request(Method.DELETE, "/networks/{id}/members/{vm_id}", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"], "vm_id": input["vm_id"]},
  });
  return z.lazy(() => NetworkInfoSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
