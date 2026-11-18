// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {EnvironmentRuleParameter} from "../models/EnvironmentRuleParameter.js";

export const EnvironmentRuleParameterSchema: z.ZodType<EnvironmentRuleParameter> = z.union([z.boolean(), z.int(), z.string(), z.array(z.string())]);
