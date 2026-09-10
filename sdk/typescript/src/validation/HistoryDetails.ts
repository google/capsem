// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {HistoryDetails} from "../models/HistoryDetails.js";
import {AuditHistoryDetailsSchema} from "./AuditHistoryDetails.js";
import {ExecHistoryDetailsSchema} from "./ExecHistoryDetails.js";

export const HistoryDetailsSchema: z.ZodType<HistoryDetails> = z.union([z.lazy(() => ExecHistoryDetailsSchema), z.lazy(() => AuditHistoryDetailsSchema)]);
