// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ImagePullRequest} from "../models/ImagePullRequest.js";
import {RegistryAccessSchema} from "./RegistryAccess.js";

export const ImagePullRequestSchema: z.ZodType<ImagePullRequest> = z.object({
  "image": z.string(),
  "registry": z.union([z.null(), z.lazy(() => RegistryAccessSchema)]).exactOptional(),
});
