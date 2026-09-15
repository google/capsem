// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {McpServersListResponse} from "../models/McpServersListResponse.js";
import {McpServerInfoResponseSchema} from "./McpServerInfoResponse.js";

export const McpServersListResponseSchema: z.ZodType<McpServersListResponse> = z.array(z.lazy(() => McpServerInfoResponseSchema));
