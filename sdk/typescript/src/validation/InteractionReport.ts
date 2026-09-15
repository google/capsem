// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {InteractionReport} from "../models/InteractionReport.js";
import {InteractionSchema} from "./Interaction.js";
import {InteractionBodySchema} from "./InteractionBody.js";

export const InteractionReportSchema: z.ZodType<InteractionReport> = z.object({
  "bodies": z.array(z.lazy(() => InteractionBodySchema)),
  "items": z.array(z.lazy(() => InteractionSchema)),
});
