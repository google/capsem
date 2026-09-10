// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {VmNetworkInfo} from "../models/VmNetworkInfo.js";

export const VmNetworkInfoSchema: z.ZodType<VmNetworkInfo> = z.object({
  "allowed_requests": z.int().min(0),
  "bytes_received": z.int().min(0),
  "bytes_sent": z.int().min(0),
  "denied_requests": z.int().min(0),
  "errors": z.int().min(0),
  "total_requests": z.int().min(0),
});
