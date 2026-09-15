// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {McpToolsListResponse} from "../models/McpToolsListResponse.js";
import {McpToolInfoResponseSchema} from "./McpToolInfoResponse.js";

export const McpToolsListResponseSchema: z.ZodType<McpToolsListResponse> = z.array(z.lazy(() => McpToolInfoResponseSchema));
