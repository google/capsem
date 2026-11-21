// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {EnvironmentPluginListResponse} from "../models/EnvironmentPluginListResponse.js";
import {EnvironmentPluginListResponseSchema} from "../validation/EnvironmentPluginListResponse.js";

export async function listNewEnvironmentPlugins(
  transport: Transport,
  options: CallOptions = {},
): Promise<EnvironmentPluginListResponse> {
  const payload = await transport.request(Method.GET, "/environments/plugins/list", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
  });
  return z.lazy(() => EnvironmentPluginListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
