// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {SnapshotsList} from "../models/SnapshotsList.js";
import {SnapshotsListSchema} from "../validation/SnapshotsList.js";

export async function listVmSnapshots(
  transport: Transport,
  parameters: {
    "id": string;
  },
  options: CallOptions = {},
): Promise<SnapshotsList> {
  const input = z.object({
  "id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/snapshots/list", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
  });
  return z.lazy(() => SnapshotsListSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
