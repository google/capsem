// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {StorageDiagnostics} from "../models/StorageDiagnostics.js";

export const StorageDiagnosticsSchema: z.ZodType<StorageDiagnostics> = z.object({
  "guest_overlay_device": z.string(),
  "guest_overlay_mount": z.string(),
  "host_available_bytes": z.int().min(0),
  "host_free_bytes": z.int().min(0),
  "host_total_bytes": z.int().min(0),
  "rootfs_image_logical_bytes": z.int().min(0),
  "rootfs_image_path": z.string(),
  "rootfs_image_physical_bytes": z.int().min(0),
});
