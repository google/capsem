// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {AuthorizationTransaction} from "../models/AuthorizationTransaction.js";
import {AuthorizationTransactionSchema} from "../validation/AuthorizationTransaction.js";

export async function getConnectionAuthorization(
  transport: Transport,
  parameters: {
    "transaction_id": string;
  },
  options: CallOptions = {},
): Promise<AuthorizationTransaction> {
  const input = z.object({
  "transaction_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/connections/authorizations/{transaction_id}", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"transaction_id": input["transaction_id"]},
  });
  return z.lazy(() => AuthorizationTransactionSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
