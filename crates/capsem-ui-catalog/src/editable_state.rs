use std::collections::BTreeMap;
use std::error::Error;

use loro::{ExportMode, LoroDoc, LoroMap, LoroMovableList, ToJson};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const EDITABLE_STATE_SCHEMA: &str = "capsem.ui.editable.v0";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableStateAudit {
    pub schema: String,
    pub base_snapshot_bytes: usize,
    pub update_bytes: usize,
    pub updated_snapshot_bytes: usize,
    pub update_blake3: String,
    pub record: EditableStateRecord,
    pub base_json: Value,
    pub updated_json: Value,
    pub restored_json: Value,
    pub topology_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableStateRecord {
    pub seq: u64,
    pub principal: String,
    pub role: String,
    pub verb: String,
    pub content_type: String,
    pub title: String,
    pub target: String,
    pub payload: EditableStatePayload,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableStatePayload {
    pub engine: String,
    pub schema: String,
    pub update_blake3: String,
    pub update_bytes: usize,
    pub affected_ids: Vec<String>,
}

pub fn run_editable_state_audit() -> Result<EditableStateAudit, Box<dyn Error>> {
    let doc = LoroDoc::new();
    doc.set_peer_id(7)?;

    let workspace = doc.get_map("workspace");
    workspace.insert("schema", EDITABLE_STATE_SCHEMA)?;
    workspace.insert("engine", "loro")?;

    let components = workspace.insert_container("components", LoroMovableList::new())?;
    let card = components.push_container(LoroMap::new())?;
    card.insert("id", "card:model")?;
    card.insert("kind", "card")?;
    card.insert("title", "Model")?;
    card.insert("body", "Gemini model card")?;
    card.insert("href", "https://gemini.google.com/")?;

    let alert = components.push_container(LoroMap::new())?;
    alert.insert("id", "alert:security-warning")?;
    alert.insert("kind", "alert")?;
    alert.insert("tone", "warning")?;
    alert.insert("message", "All your base belong to us")?;

    let deck = workspace.insert_container("slideDeck", LoroMap::new())?;
    deck.insert("id", "deck:realms-of-code")?;
    deck.insert("title", "Realms Of Code")?;
    let slides = deck.insert_container("slides", LoroMovableList::new())?;
    let overview = slides.push_container(LoroMap::new())?;
    overview.insert("id", "slide:overview")?;
    overview.insert("title", "Houses Overview")?;
    let lannister = slides.push_container(LoroMap::new())?;
    lannister.insert("id", "slide:lannister")?;
    lannister.insert("title", "House Lannister")?;

    let spreadsheet = workspace.insert_container("spreadsheet", LoroMap::new())?;
    spreadsheet.insert("id", "spreadsheet:finance")?;
    spreadsheet.insert("title", "Finance Sheet")?;
    let sheets = spreadsheet.insert_container("sheets", LoroMovableList::new())?;
    let sheet = sheets.push_container(LoroMap::new())?;
    sheet.insert("id", "sheet:houses")?;
    sheet.insert("name", "Houses")?;
    let cells = sheet.insert_container("cells", LoroMap::new())?;
    cells.insert("A1", "House")?;
    cells.insert("B1", "Motto")?;
    cells.insert("A2", "Lannister")?;
    cells.insert("B2", "Hear Me Roar")?;

    let website = workspace.insert_container("website", LoroMap::new())?;
    website.insert("id", "website:onboarding")?;
    website.insert("title", "Onboarding")?;
    let pages = website.insert_container("pages", LoroMovableList::new())?;
    let page = pages.push_container(LoroMap::new())?;
    page.insert("id", "page:welcome")?;
    page.insert("title", "Welcome")?;
    let form = page.insert_container("form", LoroMap::new())?;
    form.insert("id", "form:signup")?;
    let fields = form.insert_container("fields", LoroMovableList::new())?;
    let email = fields.push_container(LoroMap::new())?;
    email.insert("id", "field:email")?;
    email.insert("kind", "textField")?;
    email.insert("label", "Email")?;

    doc.commit();
    let base_json = doc.get_deep_value().to_json_value();
    let base_snapshot = doc.export(ExportMode::Snapshot)?;
    let base_vv = doc.oplog_vv();

    card.insert("title", "Model, live edited")?;
    cells.insert("B2", "A Lannister always pays his debts")?;
    email.insert("label", "Work email")?;
    components.mov(1, 0)?;
    slides.mov(1, 0)?;
    doc.commit();

    let update = doc.export(ExportMode::updates(&base_vv))?;
    let updated_snapshot = doc.export(ExportMode::Snapshot)?;

    let restored = LoroDoc::from_snapshot(&base_snapshot)?;
    restored.import(&update)?;
    let restored_json = restored.get_deep_value().to_json_value();
    let updated_json = doc.get_deep_value().to_json_value();

    let topology_ids = collect_ids(&updated_json);
    let update_blake3 = blake3::hash(&update).to_hex().to_string();
    let affected_ids = vec![
        "card:model".to_owned(),
        "sheet:houses".to_owned(),
        "field:email".to_owned(),
        "alert:security-warning".to_owned(),
        "slide:lannister".to_owned(),
    ];
    let record = EditableStateRecord {
        seq: 42,
        principal: "local.ui.patch".to_owned(),
        role: "tool".to_owned(),
        verb: "patch".to_owned(),
        content_type: "loroUpdate".to_owned(),
        title: "Patch editable UI state".to_owned(),
        target: "workspace".to_owned(),
        payload: EditableStatePayload {
            engine: "loro".to_owned(),
            schema: EDITABLE_STATE_SCHEMA.to_owned(),
            update_blake3: update_blake3.clone(),
            update_bytes: update.len(),
            affected_ids,
        },
    };

    Ok(EditableStateAudit {
        schema: EDITABLE_STATE_SCHEMA.to_owned(),
        base_snapshot_bytes: base_snapshot.len(),
        update_bytes: update.len(),
        updated_snapshot_bytes: updated_snapshot.len(),
        update_blake3,
        record,
        base_json,
        updated_json,
        restored_json,
        topology_ids,
    })
}

fn collect_ids(value: &Value) -> Vec<String> {
    let mut ids = BTreeMap::new();
    collect_ids_inner(value, &mut ids);
    ids.into_keys().collect()
}

fn collect_ids_inner(value: &Value, ids: &mut BTreeMap<String, ()>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(id)) = map.get("id") {
                ids.insert(id.clone(), ());
            }
            for child in map.values() {
                collect_ids_inner(child, ids);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_ids_inner(item, ids);
            }
        }
        _ => {}
    }
}

pub fn expected_audit_json() -> Value {
    json!({
        "workspace": {
            "schema": EDITABLE_STATE_SCHEMA,
            "engine": "loro",
            "components": [
                {
                    "id": "alert:security-warning",
                    "kind": "alert",
                    "tone": "warning",
                    "message": "All your base belong to us"
                },
                {
                    "id": "card:model",
                    "kind": "card",
                    "title": "Model, live edited",
                    "body": "Gemini model card",
                    "href": "https://gemini.google.com/"
                }
            ],
            "slideDeck": {
                "id": "deck:realms-of-code",
                "title": "Realms Of Code",
                "slides": [
                    {
                        "id": "slide:lannister",
                        "title": "House Lannister"
                    },
                    {
                        "id": "slide:overview",
                        "title": "Houses Overview"
                    }
                ]
            },
            "spreadsheet": {
                "id": "spreadsheet:finance",
                "title": "Finance Sheet",
                "sheets": [
                    {
                        "id": "sheet:houses",
                        "name": "Houses",
                        "cells": {
                            "A1": "House",
                            "B1": "Motto",
                            "A2": "Lannister",
                            "B2": "A Lannister always pays his debts"
                        }
                    }
                ]
            },
            "website": {
                "id": "website:onboarding",
                "title": "Onboarding",
                "pages": [
                    {
                        "id": "page:welcome",
                        "title": "Welcome",
                        "form": {
                            "id": "form:signup",
                            "fields": [
                                {
                                    "id": "field:email",
                                    "kind": "textField",
                                    "label": "Work email"
                                }
                            ]
                        }
                    }
                ]
            }
        }
    })
}
