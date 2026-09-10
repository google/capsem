import {
  FileEntryType, HostLogSource, ToolDecision,
  type ExecRequest, type FileListEntry, type TimelineStatus, type UpdateApplyRequest,
  type VmStatsDetailResponse,
} from '../src/models/index.js';

const request: UpdateApplyRequest = {};
const status: TimelineStatus = ToolDecision.DENIED;
const numeric: TimelineStatus = 403;
const leaf: FileListEntry = {
  name: 'large.bin', path: '/large.bin', type: FileEntryType.FILE,
  size: 4294967296, mtime: 0, children: null,
};
const tree: FileListEntry = { ...leaf, type: FileEntryType.DIRECTORY, children: [leaf] };
const bodies: VmStatsDetailResponse['body_blobs'] = { event: [] };

// @ts-expect-error An optional boolean cannot be explicit null.
request.confirmed = null;
// @ts-expect-error Exact optional fields cannot be explicitly undefined.
request.confirmed = undefined;
// @ts-expect-error Enums require named members, not magic strings.
const magic: HostLogSource = 'service';
// @ts-expect-error A numeric status does not accept numeric strings.
const badStatus: TimelineStatus = '403';
// @ts-expect-error Arrays retain their element type.
tree.children = [1];
// @ts-expect-error Required execution fields cannot be omitted.
const incomplete: ExecRequest = {};
// @ts-expect-error Counters cannot become strings.
leaf.size = '4294967296';

if (numeric !== 403 || status !== ToolDecision.DENIED || !Array.isArray(bodies.event)) {
  throw new Error('Generated type usage failed');
}
void magic; void badStatus; void incomplete;
