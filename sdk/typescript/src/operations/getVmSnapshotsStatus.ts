// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {SnapshotsStatus} from "../models/SnapshotsStatus.js";
import {SnapshotsStatusSchema} from "../validation/SnapshotsStatus.js";

export async function getVmSnapshotsStatus(
  transport: Transport,
  parameters: {
    "id": string;
  },
  options: CallOptions = {},
): Promise<SnapshotsStatus> {
  const input = z.object({
  "id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/snapshots/status", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
  });
  return z.lazy(() => SnapshotsStatusSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
