use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::native_deck::NativeArtifact;

pub const ROOT_SLOT: &str = "chat";
const ALLOWED_STYLE_PROPERTIES: &[&str] = &[
    "backgroundColor",
    "color",
    "fontStyle",
    "fontWeight",
    "opacity",
    "textDecoration",
];
const MAX_STYLE_VALUE_BYTES: usize = 120;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRecord {
    pub seq: u64,
    pub id: String,
    pub timestamp: String,
    pub role: WorkspaceRole,
    pub principal: String,
    pub title: String,
    pub content_type: WorkspaceContentType,
    pub verb: WorkspaceVerb,
    pub status: WorkspaceStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub content: WorkspaceContent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceRole {
    User,
    Assistant,
    Tool,
    Plugin,
    System,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceContentType {
    Artifact,
    Text,
    Ui,
    ToolCall,
    Action,
    ElementPatch,
    Status,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceVerb {
    Create,
    Append,
    Replace,
    Patch,
    Delete,
    Select,
    Request,
    Respond,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceStatus {
    Pending,
    Running,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WorkspaceContent {
    Artifact {
        artifact: NativeArtifact,
    },
    Text {
        text: String,
    },
    Ui {
        spec: Value,
    },
    ToolCall {
        name: String,
        input: Value,
        output: Option<Value>,
    },
    Action {
        name: String,
        payload: Value,
    },
    ElementPatch {
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, rename = "textPatches", skip_serializing_if = "Vec::is_empty")]
        text_patches: Vec<ElementTextPatch>,
        #[serde(
            default,
            rename = "stylePatches",
            skip_serializing_if = "Vec::is_empty"
        )]
        style_patches: Vec<ElementStylePatch>,
    },
    Status {
        message: String,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementStylePatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow_selector: Option<String>,
    pub styles: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_request_seq: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementTextPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow_selector: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_request_seq: Option<u64>,
}

impl WorkspaceContent {
    pub fn content_type(&self) -> WorkspaceContentType {
        match self {
            Self::Artifact { .. } => WorkspaceContentType::Artifact,
            Self::Text { .. } => WorkspaceContentType::Text,
            Self::Ui { .. } => WorkspaceContentType::Ui,
            Self::ToolCall { .. } => WorkspaceContentType::ToolCall,
            Self::Action { .. } => WorkspaceContentType::Action,
            Self::ElementPatch { .. } => WorkspaceContentType::ElementPatch,
            Self::Status { .. } => WorkspaceContentType::Status,
            Self::Error { .. } => WorkspaceContentType::Error,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFrame {
    pub record: WorkspaceRecord,
    pub deltas: Vec<RenderDelta>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderProjection {
    pub seq: u64,
    pub elements: BTreeMap<String, RenderElement>,
    pub topology: RenderTopology,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tasks: BTreeMap<String, WorkspaceTask>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderTopology {
    pub roots: Vec<String>,
    pub nodes: BTreeMap<String, TopologyNode>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyNode {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub slot: String,
    pub index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceTaskStatus {
    Open,
    Resolved,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTask {
    pub id: String,
    pub target: String,
    pub instruction: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation: Option<WorkspaceAnnotationTarget>,
    pub status: WorkspaceTaskStatus,
    pub created_seq: u64,
    pub created_at: String,
    pub created_by: String,
    pub updated_seq: u64,
    pub updated_at: String,
    pub updated_by: String,
    pub source_record_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderElement {
    pub id: String,
    pub title: String,
    pub content_type: WorkspaceContentType,
    pub status: WorkspaceStatus,
    pub component: String,
    pub provenance: ElementProvenance,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<NativeArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementProvenance {
    pub created_seq: u64,
    pub created_at: String,
    pub created_by: String,
    pub updated_seq: u64,
    pub updated_at: String,
    pub updated_by: String,
    pub last_record_id: String,
    pub last_verb: WorkspaceVerb,
}

impl ElementProvenance {
    fn from_record(record: &WorkspaceRecord) -> Self {
        Self {
            created_seq: record.seq,
            created_at: record.timestamp.clone(),
            created_by: record.principal.clone(),
            updated_seq: record.seq,
            updated_at: record.timestamp.clone(),
            updated_by: record.principal.clone(),
            last_record_id: record.id.clone(),
            last_verb: record.verb,
        }
    }

    fn updated_from(mut self, record: &WorkspaceRecord) -> Self {
        self.updated_seq = record.seq;
        self.updated_at = record.timestamp.clone();
        self.updated_by = record.principal.clone();
        self.last_record_id = record.id.clone();
        self.last_verb = record.verb;
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RenderDelta {
    UpsertElement { id: String, element: RenderElement },
    DeleteElement { id: String },
    UpsertTopologyNode { node: TopologyNode },
    DeleteTopologyNode { id: String },
    UpsertTask { id: String, task: WorkspaceTask },
    Select { id: Option<String> },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCheckpoint {
    pub checkpoint_seq: u64,
    pub created_at: String,
    pub workspace_id: String,
    pub projection_version: u32,
    pub records_compacted: Option<CompactedRange>,
    pub projection: RenderProjection,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactedRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshot {
    pub workspace_id: String,
    pub checkpoint: WorkspaceCheckpoint,
    pub projection: RenderProjection,
    pub tail: Vec<WorkspaceFrame>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceChangeRequest {
    pub instruction: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation: Option<WorkspaceAnnotationTarget>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTaskResolution {
    pub task_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceAnnotationTarget {
    pub kind: String,
    pub label: String,
    pub path: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topology_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topology_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow_selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default)]
pub struct WorkspaceProjector {
    projection: RenderProjection,
}

impl WorkspaceProjector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_projection(projection: RenderProjection) -> Self {
        Self { projection }
    }

    pub fn from_checkpoint(checkpoint: WorkspaceCheckpoint) -> Self {
        Self::from_projection(checkpoint.projection)
    }

    pub fn projection(&self) -> &RenderProjection {
        &self.projection
    }

    pub fn apply(&mut self, record: &WorkspaceRecord) -> Result<Vec<RenderDelta>, String> {
        if record.content_type != record.content.content_type() {
            return Err("record content_type does not match content variant".to_owned());
        }

        self.projection.seq = record.seq;
        match record.verb {
            WorkspaceVerb::Create | WorkspaceVerb::Replace => self.upsert(record),
            WorkspaceVerb::Patch => self.patch(record),
            WorkspaceVerb::Delete => self.delete(record),
            WorkspaceVerb::Select => self.select(record),
            WorkspaceVerb::Request => self.request(record),
            WorkspaceVerb::Respond => self.respond(record),
            WorkspaceVerb::Append => Ok(vec![]),
        }
    }

    pub fn checkpoint(
        &self,
        workspace_id: impl Into<String>,
        created_at: String,
    ) -> WorkspaceCheckpoint {
        WorkspaceCheckpoint {
            checkpoint_seq: self.projection.seq,
            created_at,
            workspace_id: workspace_id.into(),
            projection_version: 1,
            records_compacted: if self.projection.seq == 0 {
                None
            } else {
                Some(CompactedRange {
                    start: 1,
                    end: self.projection.seq,
                })
            },
            projection: self.projection.clone(),
        }
    }

    fn upsert(&mut self, record: &WorkspaceRecord) -> Result<Vec<RenderDelta>, String> {
        let element = element_from_record(record)?;
        let id = element.id.clone();
        let existing = self.projection.elements.get(&id);
        let is_new = existing.is_none();
        let mut element = element;
        if let Some(existing) = existing {
            element.provenance = existing.provenance.clone().updated_from(record);
        }
        self.projection.elements.insert(id.clone(), element.clone());

        let mut deltas = vec![RenderDelta::UpsertElement {
            id: id.clone(),
            element,
        }];
        if is_new {
            let node = TopologyNode {
                id: id.clone(),
                parent: None,
                slot: ROOT_SLOT.to_owned(),
                index: self.projection.topology.roots.len(),
            };
            self.projection.topology.roots.push(id.clone());
            self.projection
                .topology
                .nodes
                .insert(id.clone(), node.clone());
            deltas.push(RenderDelta::UpsertTopologyNode { node });
        }
        if self.projection.selected.is_none() {
            self.projection.selected = Some(id.clone());
            deltas.push(RenderDelta::Select { id: Some(id) });
        }
        Ok(deltas)
    }

    fn delete(&mut self, record: &WorkspaceRecord) -> Result<Vec<RenderDelta>, String> {
        let target = record
            .target
            .as_deref()
            .ok_or_else(|| "delete record requires target".to_owned())?;
        let mut removed = self.topology_subtree(target);
        if removed.is_empty() {
            removed.push(target.to_owned());
        }
        let mut deltas = Vec::new();
        for id in &removed {
            self.projection.elements.remove(id);
            self.projection.topology.nodes.remove(id);
            self.projection.topology.roots.retain(|root| root != id);
            deltas.push(RenderDelta::DeleteElement { id: id.clone() });
            deltas.push(RenderDelta::DeleteTopologyNode { id: id.clone() });
        }
        normalize_root_indexes(&mut self.projection.topology);
        if self.projection.selected.as_deref() == Some(target) {
            self.projection.selected = self.projection.topology.roots.first().cloned();
            deltas.push(RenderDelta::Select {
                id: self.projection.selected.clone(),
            });
        }
        Ok(deltas)
    }

    fn patch(&mut self, record: &WorkspaceRecord) -> Result<Vec<RenderDelta>, String> {
        let target = record
            .target
            .as_deref()
            .ok_or_else(|| "patch record requires target".to_owned())?;
        let mut element = self
            .projection
            .elements
            .get(target)
            .cloned()
            .ok_or_else(|| format!("cannot patch missing element: {target}"))?;
        match &record.content {
            WorkspaceContent::ElementPatch {
                title,
                text,
                text_patches,
                style_patches,
            } => {
                if let Some(title) = title {
                    if title.trim().is_empty() {
                        return Err("patch title must not be empty".to_owned());
                    }
                    element.title = title.clone();
                }
                if let Some(text) = text {
                    if element.content_type != WorkspaceContentType::Text {
                        return Err("text patch requires text element".to_owned());
                    }
                    if text.trim().is_empty() {
                        return Err("patch text must not be empty".to_owned());
                    }
                    element.content = Some(serde_json::json!({ "text": text }));
                }
                if !style_patches.is_empty() {
                    apply_style_patches(&mut element, style_patches)?;
                }
                if !text_patches.is_empty() {
                    apply_text_patches(&mut element, text_patches)?;
                }
            }
            _ => return Err("patch record requires elementPatch content".to_owned()),
        }
        element.status = record.status;
        element.provenance = element.provenance.updated_from(record);
        self.projection
            .elements
            .insert(target.to_owned(), element.clone());
        Ok(vec![RenderDelta::UpsertElement {
            id: target.to_owned(),
            element,
        }])
    }

    fn select(&mut self, record: &WorkspaceRecord) -> Result<Vec<RenderDelta>, String> {
        let target = record.target.clone().or_else(|| match &record.content {
            WorkspaceContent::Artifact { artifact } => Some(artifact.id.clone()),
            _ => None,
        });
        if let Some(id) = target.as_deref() {
            if !self.projection.elements.contains_key(id) {
                return Err(format!("cannot select missing element: {id}"));
            }
        }
        self.projection.selected = target.clone();
        Ok(vec![RenderDelta::Select { id: target }])
    }

    fn request(&mut self, record: &WorkspaceRecord) -> Result<Vec<RenderDelta>, String> {
        let request = match &record.content {
            WorkspaceContent::Action { name, payload } if name == "ui.change" => {
                serde_json::from_value::<WorkspaceChangeRequest>(payload.clone())
                    .map_err(|err| format!("invalid ui.change payload: {err}"))?
            }
            _ => return Ok(vec![]),
        };
        let target = record
            .target
            .as_deref()
            .ok_or_else(|| "change request requires target".to_owned())?;
        if !self.projection.elements.contains_key(target) {
            return Err(format!(
                "cannot request change for missing element: {target}"
            ));
        }

        let task = WorkspaceTask {
            id: task_id_from_record(record),
            target: target.to_owned(),
            instruction: request.instruction,
            annotation: request.annotation,
            status: WorkspaceTaskStatus::Open,
            created_seq: record.seq,
            created_at: record.timestamp.clone(),
            created_by: record.principal.clone(),
            updated_seq: record.seq,
            updated_at: record.timestamp.clone(),
            updated_by: record.principal.clone(),
            source_record_id: record.id.clone(),
            resolved_seq: None,
            resolved_at: None,
            resolved_by: None,
        };
        let id = task.id.clone();
        self.projection.tasks.insert(id.clone(), task.clone());
        Ok(vec![RenderDelta::UpsertTask { id, task }])
    }

    fn respond(&mut self, record: &WorkspaceRecord) -> Result<Vec<RenderDelta>, String> {
        let resolution = match &record.content {
            WorkspaceContent::Action { name, payload } if name == "ui.resolve" => {
                serde_json::from_value::<WorkspaceTaskResolution>(payload.clone())
                    .map_err(|err| format!("invalid ui.resolve payload: {err}"))?
            }
            _ => return Ok(vec![]),
        };
        let task = self
            .projection
            .tasks
            .get_mut(&resolution.task_id)
            .ok_or_else(|| format!("cannot resolve missing task: {}", resolution.task_id))?;
        task.status = WorkspaceTaskStatus::Resolved;
        task.updated_seq = record.seq;
        task.updated_at = record.timestamp.clone();
        task.updated_by = record.principal.clone();
        task.resolved_seq = Some(record.seq);
        task.resolved_at = Some(record.timestamp.clone());
        task.resolved_by = Some(record.principal.clone());
        let task = task.clone();
        Ok(vec![RenderDelta::UpsertTask {
            id: resolution.task_id,
            task,
        }])
    }

    fn topology_subtree(&self, target: &str) -> Vec<String> {
        let mut removed = vec![target.to_owned()];
        let mut cursor = 0;
        while cursor < removed.len() {
            let parent = removed[cursor].clone();
            let children: Vec<String> = self
                .projection
                .topology
                .nodes
                .values()
                .filter(|node| node.parent.as_deref() == Some(parent.as_str()))
                .map(|node| node.id.clone())
                .collect();
            for child in children {
                if !removed.iter().any(|id| id == &child) {
                    removed.push(child);
                }
            }
            cursor += 1;
        }
        removed
    }
}

fn normalize_root_indexes(topology: &mut RenderTopology) {
    for (index, id) in topology.roots.iter().enumerate() {
        if let Some(node) = topology.nodes.get_mut(id) {
            node.index = index;
        }
    }
}

pub fn artifact_record(
    seq: u64,
    timestamp: String,
    principal: impl Into<String>,
    verb: WorkspaceVerb,
    artifact: NativeArtifact,
) -> WorkspaceRecord {
    WorkspaceRecord {
        seq,
        id: format!("rec-{seq}"),
        timestamp,
        role: WorkspaceRole::Tool,
        principal: principal.into(),
        title: artifact.title.clone(),
        content_type: WorkspaceContentType::Artifact,
        verb,
        status: artifact_status(&artifact),
        target: Some(artifact.id.clone()),
        content: WorkspaceContent::Artifact { artifact },
    }
}

pub fn delete_artifact_record(
    seq: u64,
    timestamp: String,
    principal: impl Into<String>,
    artifact: NativeArtifact,
) -> WorkspaceRecord {
    WorkspaceRecord {
        seq,
        id: format!("rec-{seq}"),
        timestamp,
        role: WorkspaceRole::Tool,
        principal: principal.into(),
        title: format!("Delete {}", artifact.title),
        content_type: WorkspaceContentType::Artifact,
        verb: WorkspaceVerb::Delete,
        status: WorkspaceStatus::Complete,
        target: Some(artifact.id.clone()),
        content: WorkspaceContent::Artifact { artifact },
    }
}

pub fn select_record(
    seq: u64,
    timestamp: String,
    principal: impl Into<String>,
    target: Option<String>,
) -> WorkspaceRecord {
    let title = target
        .as_deref()
        .map(|id| format!("Select {id}"))
        .unwrap_or_else(|| "Clear selection".to_owned());
    let message = title.clone();
    WorkspaceRecord {
        seq,
        id: format!("rec-{seq}"),
        timestamp,
        role: WorkspaceRole::Tool,
        principal: principal.into(),
        title,
        content_type: WorkspaceContentType::Status,
        verb: WorkspaceVerb::Select,
        status: WorkspaceStatus::Complete,
        target,
        content: WorkspaceContent::Status { message },
    }
}

pub fn text_record(
    seq: u64,
    timestamp: String,
    principal: impl Into<String>,
    target: String,
    title: String,
    text: String,
) -> WorkspaceRecord {
    WorkspaceRecord {
        seq,
        id: format!("rec-{seq}"),
        timestamp,
        role: WorkspaceRole::Tool,
        principal: principal.into(),
        title,
        content_type: WorkspaceContentType::Text,
        verb: WorkspaceVerb::Create,
        status: WorkspaceStatus::Complete,
        target: Some(target),
        content: WorkspaceContent::Text { text },
    }
}

pub fn title_patch_record(
    seq: u64,
    timestamp: String,
    principal: impl Into<String>,
    target: String,
    title: String,
) -> WorkspaceRecord {
    WorkspaceRecord {
        seq,
        id: format!("rec-{seq}"),
        timestamp,
        role: WorkspaceRole::Tool,
        principal: principal.into(),
        title: format!("Rename {target}"),
        content_type: WorkspaceContentType::ElementPatch,
        verb: WorkspaceVerb::Patch,
        status: WorkspaceStatus::Complete,
        target: Some(target),
        content: WorkspaceContent::ElementPatch {
            title: Some(title),
            text: None,
            text_patches: vec![],
            style_patches: vec![],
        },
    }
}

pub fn text_patch_record(
    seq: u64,
    timestamp: String,
    principal: impl Into<String>,
    target: String,
    text: String,
) -> WorkspaceRecord {
    WorkspaceRecord {
        seq,
        id: format!("rec-{seq}"),
        timestamp,
        role: WorkspaceRole::Tool,
        principal: principal.into(),
        title: format!("Patch text {target}"),
        content_type: WorkspaceContentType::ElementPatch,
        verb: WorkspaceVerb::Patch,
        status: WorkspaceStatus::Complete,
        target: Some(target),
        content: WorkspaceContent::ElementPatch {
            title: None,
            text: Some(text),
            text_patches: vec![],
            style_patches: vec![],
        },
    }
}

pub fn text_selector_patch_record(
    seq: u64,
    timestamp: String,
    principal: impl Into<String>,
    target: String,
    text_patch: ElementTextPatch,
) -> WorkspaceRecord {
    WorkspaceRecord {
        seq,
        id: format!("rec-{seq}"),
        timestamp,
        role: WorkspaceRole::Tool,
        principal: principal.into(),
        title: format!("Patch text {target}"),
        content_type: WorkspaceContentType::ElementPatch,
        verb: WorkspaceVerb::Patch,
        status: WorkspaceStatus::Complete,
        target: Some(target),
        content: WorkspaceContent::ElementPatch {
            title: None,
            text: None,
            text_patches: vec![text_patch],
            style_patches: vec![],
        },
    }
}

pub fn style_patch_record(
    seq: u64,
    timestamp: String,
    principal: impl Into<String>,
    target: String,
    style_patch: ElementStylePatch,
) -> WorkspaceRecord {
    WorkspaceRecord {
        seq,
        id: format!("rec-{seq}"),
        timestamp,
        role: WorkspaceRole::Tool,
        principal: principal.into(),
        title: format!("Patch style {target}"),
        content_type: WorkspaceContentType::ElementPatch,
        verb: WorkspaceVerb::Patch,
        status: WorkspaceStatus::Complete,
        target: Some(target),
        content: WorkspaceContent::ElementPatch {
            title: None,
            text: None,
            text_patches: vec![],
            style_patches: vec![style_patch],
        },
    }
}

pub fn change_request_record(
    seq: u64,
    timestamp: String,
    principal: impl Into<String>,
    target: String,
    instruction: String,
    annotation: Option<WorkspaceAnnotationTarget>,
) -> WorkspaceRecord {
    WorkspaceRecord {
        seq,
        id: format!("rec-{seq}"),
        timestamp,
        role: WorkspaceRole::User,
        principal: principal.into(),
        title: format!("Change {target}"),
        content_type: WorkspaceContentType::Action,
        verb: WorkspaceVerb::Request,
        status: WorkspaceStatus::Pending,
        target: Some(target),
        content: WorkspaceContent::Action {
            name: "ui.change".to_owned(),
            payload: serde_json::to_value(WorkspaceChangeRequest {
                instruction,
                annotation,
            })
            .expect("workspace change request serializes"),
        },
    }
}

pub fn resolve_task_record(
    seq: u64,
    timestamp: String,
    principal: impl Into<String>,
    task_id: String,
) -> WorkspaceRecord {
    WorkspaceRecord {
        seq,
        id: format!("rec-{seq}"),
        timestamp,
        role: WorkspaceRole::Assistant,
        principal: principal.into(),
        title: format!("Resolve {task_id}"),
        content_type: WorkspaceContentType::Action,
        verb: WorkspaceVerb::Respond,
        status: WorkspaceStatus::Complete,
        target: Some(task_id.clone()),
        content: WorkspaceContent::Action {
            name: "ui.resolve".to_owned(),
            payload: serde_json::to_value(WorkspaceTaskResolution { task_id })
                .expect("workspace task resolution serializes"),
        },
    }
}

pub fn change_request_from_record(record: &WorkspaceRecord) -> Option<WorkspaceChangeRequest> {
    match &record.content {
        WorkspaceContent::Action { name, payload } if name == "ui.change" => {
            serde_json::from_value(payload.clone()).ok()
        }
        _ => None,
    }
}

pub fn resolve_task_from_record(record: &WorkspaceRecord) -> Option<WorkspaceTaskResolution> {
    match &record.content {
        WorkspaceContent::Action { name, payload } if name == "ui.resolve" => {
            serde_json::from_value(payload.clone()).ok()
        }
        _ => None,
    }
}

pub fn task_id_from_record(record: &WorkspaceRecord) -> String {
    format!("task-{}", record.seq)
}

fn element_from_record(record: &WorkspaceRecord) -> Result<RenderElement, String> {
    match &record.content {
        WorkspaceContent::Artifact { artifact } => Ok(RenderElement {
            id: record.target.clone().unwrap_or_else(|| artifact.id.clone()),
            title: record.title.clone(),
            content_type: record.content_type,
            status: record.status,
            component: artifact.spec["component"]
                .as_str()
                .unwrap_or("capsem-elt")
                .to_owned(),
            provenance: ElementProvenance::from_record(record),
            artifact: Some(artifact.clone()),
            content: None,
        }),
        WorkspaceContent::Text { text } => Ok(RenderElement {
            id: required_target(record)?,
            title: record.title.clone(),
            content_type: record.content_type,
            status: record.status,
            component: "capsem-text".to_owned(),
            provenance: ElementProvenance::from_record(record),
            artifact: None,
            content: Some(serde_json::json!({ "text": text })),
        }),
        WorkspaceContent::Ui { spec } => Ok(RenderElement {
            id: required_target(record)?,
            title: record.title.clone(),
            content_type: record.content_type,
            status: record.status,
            component: spec["component"]
                .as_str()
                .unwrap_or("capsem-elt")
                .to_owned(),
            provenance: ElementProvenance::from_record(record),
            artifact: None,
            content: Some(spec.clone()),
        }),
        _ => Err("record content cannot be projected into a render element".to_owned()),
    }
}

fn required_target(record: &WorkspaceRecord) -> Result<String, String> {
    record
        .target
        .clone()
        .ok_or_else(|| "projected record requires target".to_owned())
}

fn apply_style_patches(
    element: &mut RenderElement,
    style_patches: &[ElementStylePatch],
) -> Result<(), String> {
    for patch in style_patches {
        validate_style_patch(patch)?;
    }
    let patch_values: Vec<Value> = style_patches
        .iter()
        .map(|patch| serde_json::to_value(patch).expect("style patch serializes"))
        .collect();

    if let Some(artifact) = element.artifact.as_mut() {
        let existing = artifact
            .spec
            .get("stylePatches")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut next = existing;
        for patch in patch_values {
            merge_style_patch(&mut next, patch);
        }
        artifact.spec["stylePatches"] = Value::Array(next);
        return Ok(());
    }

    let mut content = element
        .content
        .take()
        .unwrap_or_else(|| serde_json::json!({}));
    let object = content
        .as_object_mut()
        .ok_or_else(|| "element content cannot receive style patches".to_owned())?;
    let existing = object
        .get("stylePatches")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut next = existing;
    for patch in patch_values {
        merge_style_patch(&mut next, patch);
    }
    object.insert("stylePatches".to_owned(), Value::Array(next));
    element.content = Some(content);
    Ok(())
}

fn apply_text_patches(
    element: &mut RenderElement,
    text_patches: &[ElementTextPatch],
) -> Result<(), String> {
    for patch in text_patches {
        validate_text_patch(patch)?;
    }
    let patch_values: Vec<Value> = text_patches
        .iter()
        .map(|patch| serde_json::to_value(patch).expect("text patch serializes"))
        .collect();

    if let Some(artifact) = element.artifact.as_mut() {
        append_patch_values(&mut artifact.spec, "textPatches", patch_values)?;
        return Ok(());
    }

    let mut content = element
        .content
        .take()
        .unwrap_or_else(|| serde_json::json!({}));
    append_patch_values(&mut content, "textPatches", patch_values)?;
    element.content = Some(content);
    Ok(())
}

fn append_patch_values(
    target: &mut Value,
    key: &str,
    patch_values: Vec<Value>,
) -> Result<(), String> {
    let object = target
        .as_object_mut()
        .ok_or_else(|| format!("element target cannot receive {key}"))?;
    let existing = object
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut next = existing;
    next.extend(patch_values);
    object.insert(key.to_owned(), Value::Array(next));
    Ok(())
}

fn validate_text_patch(patch: &ElementTextPatch) -> Result<(), String> {
    if patch.text.trim().is_empty()
        || (patch.selector.as_deref().unwrap_or("").is_empty()
            && patch.shadow_selector.as_deref().unwrap_or("").is_empty())
    {
        return Err("text patch requires non-empty selector or shadowSelector and text".to_owned());
    }
    let stable_selector = patch
        .shadow_selector
        .as_deref()
        .or(patch.selector.as_deref())
        .unwrap_or_default();
    if !stable_selector.contains("data-capsem-node=")
        && !stable_selector.contains("data-capsem-topology-id=")
    {
        return Err("text patch selector must target a stable Capsem topology node".to_owned());
    }
    Ok(())
}

fn validate_style_patch(patch: &ElementStylePatch) -> Result<(), String> {
    if patch.styles.is_empty()
        || (patch.selector.as_deref().unwrap_or("").is_empty()
            && patch.shadow_selector.as_deref().unwrap_or("").is_empty())
    {
        return Err(
            "style patch requires non-empty selector or shadowSelector and styles".to_owned(),
        );
    }
    for (name, value) in &patch.styles {
        if !ALLOWED_STYLE_PROPERTIES.contains(&name.as_str()) {
            return Err(format!("style property is not allowed: {name}"));
        }
        validate_style_value(name, value)?;
    }
    Ok(())
}

fn validate_style_value(name: &str, value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("style value must not be empty: {name}"));
    }
    if trimmed.len() > MAX_STYLE_VALUE_BYTES {
        return Err(format!("style value is too long: {name}"));
    }
    let lowered = trimmed.to_ascii_lowercase();
    if trimmed.contains(';')
        || trimmed.contains('{')
        || trimmed.contains('}')
        || lowered.contains("url(")
        || lowered.contains("expression(")
        || lowered.contains("@import")
        || lowered.contains("!important")
    {
        return Err(format!("style value is not allowed: {name}"));
    }
    Ok(())
}

fn merge_style_patch(existing: &mut Vec<Value>, patch: Value) {
    let selector = patch
        .get("selector")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let shadow_selector = patch
        .get("shadowSelector")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let Some(shadow_selector) = shadow_selector {
        if let Some(slot) = existing.iter_mut().find(|item| {
            item.get("shadowSelector")
                .and_then(Value::as_str)
                .is_some_and(|selector| selector == shadow_selector)
        }) {
            *slot = patch;
            return;
        }
    }
    if let Some(selector) = selector {
        if let Some(slot) = existing.iter_mut().find(|item| {
            item.get("selector")
                .and_then(Value::as_str)
                .is_some_and(|item_selector| item_selector == selector)
        }) {
            *slot = patch;
            return;
        }
    }
    existing.push(patch);
}

fn artifact_status(artifact: &NativeArtifact) -> WorkspaceStatus {
    match artifact.spec["status"].as_str() {
        Some("planned") | Some("running") => WorkspaceStatus::Running,
        Some("providerError") | Some("configMissing") => WorkspaceStatus::Failed,
        Some("cancelled") => WorkspaceStatus::Cancelled,
        _ => WorkspaceStatus::Complete,
    }
}
