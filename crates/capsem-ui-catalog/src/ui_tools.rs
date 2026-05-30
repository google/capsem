use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ui::{
    validate_messages, A2uiServerMessage, A2uiVersion, Action, Align, BasicComponent, Button,
    ButtonVariant, Card, ChildList, Column, CreateSurface, DynamicString, EventAction, Icon,
    IconName, KnownIcon, Modal, Row, Text, TextVariant, UpdateComponents, A2UI_BASIC_CATALOG_ID,
};

pub const CAPSEM_UI_CATALOG_ID: &str = "https://capsem.org/schemas/ui/catalog.v1.json";
pub const TYPED_UI_RECIPE_COMPONENTS: &[&str] = &["alert", "card", "modal", "table"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiToolProgram {
    pub calls: Vec<UiToolCall>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiToolCall {
    pub tool: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiToolProgramResult {
    pub ok: bool,
    pub observations: Vec<UiToolObservation>,
    pub surfaces: Vec<UiToolSurfaceSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiToolObservation {
    pub tool: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<A2uiServerMessage>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiToolSurfaceSnapshot {
    pub surface_id: String,
    pub catalog_id: String,
    pub component_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipe: Option<UiToolRecipe>,
    pub messages: Vec<A2uiServerMessage>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiToolRecipe {
    pub component: String,
    pub variant: String,
    pub docs_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<UiToolLink>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiToolLink {
    pub label: String,
    pub href: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateSpec {
    pub schema: String,
    pub component: String,
    pub variant: String,
    #[serde(default)]
    pub structural: Vec<String>,
    #[serde(default)]
    pub bindings: Vec<TemplateBinding>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateBinding {
    pub prop: String,
    pub kind: TemplateBindingKind,
    pub selector: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateBindingKind {
    Text,
    Attribute,
    ComponentRef,
    Action,
    State,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateCheckReport {
    pub ok: bool,
    pub component: String,
    pub variant: String,
    pub errors: Vec<String>,
}

#[derive(Default)]
pub struct UiToolRunner {
    surfaces: BTreeMap<String, UiToolSurface>,
}

#[derive(Clone, Debug)]
struct UiToolSurface {
    catalog_id: String,
    recipe: Option<UiToolRecipe>,
    components: BTreeMap<String, BasicComponent>,
}

impl UiToolRunner {
    pub fn run(mut self, program: UiToolProgram) -> UiToolProgramResult {
        let mut observations = Vec::new();
        for call in program.calls {
            observations.push(self.call(call));
        }
        let ok = observations.iter().all(|observation| observation.ok);
        let surfaces = self
            .surfaces
            .iter()
            .map(|(surface_id, surface)| UiToolSurfaceSnapshot {
                surface_id: surface_id.clone(),
                catalog_id: surface.catalog_id.clone(),
                component_count: surface.components.len(),
                recipe: surface.recipe.clone(),
                messages: surface.messages(surface_id),
            })
            .collect();

        UiToolProgramResult {
            ok,
            observations,
            surfaces,
        }
    }

    fn call(&mut self, call: UiToolCall) -> UiToolObservation {
        match call.tool.as_str() {
            "ui.catalog.list" => ok(call.tool, "catalogs listed")
                .with_messages(Vec::new())
                .with_surface_id(CAPSEM_UI_CATALOG_ID),
            "ui.catalog.describe" => self.catalog_describe(call),
            "ui.surface.create" => self.surface_create(call),
            "ui.component.add" => self.component_add(call),
            "ui.surface.validate" => self.surface_validate(call),
            "ui.surface.preview" => self.surface_preview(call),
            "ui.surface.clear" => self.surface_clear(call),
            "ui.alert" => self.alert(call),
            "ui.card" => self.card(call),
            "ui.table" => self.table(call),
            "ui.ask" => self.ask(call),
            other => err(other, vec![format!("unknown UI tool: {other}")]),
        }
    }

    fn catalog_describe(&self, call: UiToolCall) -> UiToolObservation {
        let component = string_arg(&call.args, "component");
        match component.as_deref() {
            Some("Card" | "Text" | "Button" | "Modal" | "Row" | "Column" | "Alert") => {
                ok(call.tool, "component described").with_component_id(component.unwrap())
            }
            Some(name) => err(
                call.tool,
                vec![format!("unknown catalog component: {name}")],
            ),
            None => err(call.tool, vec!["component is required".to_owned()]),
        }
    }

    fn surface_create(&mut self, call: UiToolCall) -> UiToolObservation {
        let Some(surface_id) = string_arg(&call.args, "id") else {
            return err(call.tool, vec!["id is required".to_owned()]);
        };
        if surface_id.trim().is_empty() {
            return err(call.tool, vec!["id must be non-empty".to_owned()]);
        }
        let catalog_id =
            string_arg(&call.args, "catalog").unwrap_or_else(|| A2UI_BASIC_CATALOG_ID.to_owned());
        if catalog_id != A2UI_BASIC_CATALOG_ID && catalog_id != CAPSEM_UI_CATALOG_ID {
            return err(
                call.tool,
                vec![format!("unsupported catalog: {catalog_id}")],
            );
        }
        self.surfaces.insert(
            surface_id.clone(),
            UiToolSurface {
                catalog_id,
                recipe: None,
                components: BTreeMap::new(),
            },
        );
        ok(call.tool, "surface created").with_surface_id(surface_id)
    }

    fn component_add(&mut self, call: UiToolCall) -> UiToolObservation {
        let Some(surface_id) = string_arg(&call.args, "surfaceId") else {
            return err(call.tool, vec!["surfaceId is required".to_owned()]);
        };
        let Some(surface) = self.surfaces.get_mut(&surface_id) else {
            return err(call.tool, vec![format!("surface not found: {surface_id}")]);
        };
        let Some(component_value) = call.args.get("component").cloned() else {
            return err(call.tool, vec!["component is required".to_owned()]);
        };
        if contains_forbidden_renderer_input(&component_value) {
            return err(
                call.tool,
                vec![
                    "component cannot contain raw html, class strings, or renderer code".to_owned(),
                ],
            );
        }
        let component = match serde_json::from_value::<BasicComponent>(component_value) {
            Ok(component) => component,
            Err(error) => return err(call.tool, vec![error.to_string()]),
        };
        let component_id = component.id().to_owned();
        surface.components.insert(component_id.clone(), component);
        ok(call.tool, "component added")
            .with_surface_id(surface_id)
            .with_component_id(component_id)
    }

    fn surface_validate(&mut self, call: UiToolCall) -> UiToolObservation {
        let Some(surface_id) = string_arg(&call.args, "surfaceId") else {
            return err(call.tool, vec!["surfaceId is required".to_owned()]);
        };
        let Some(surface) = self.surfaces.get(&surface_id) else {
            return err(call.tool, vec![format!("surface not found: {surface_id}")]);
        };
        let messages = surface.messages(&surface_id);
        match validate_messages(&messages) {
            Ok(()) => ok(call.tool, "surface valid")
                .with_surface_id(surface_id)
                .with_messages(messages),
            Err(error) => err(call.tool, vec![error]).with_surface_id(surface_id),
        }
    }

    fn surface_preview(&mut self, call: UiToolCall) -> UiToolObservation {
        let Some(surface_id) = string_arg(&call.args, "surfaceId") else {
            return err(call.tool, vec!["surfaceId is required".to_owned()]);
        };
        let Some(surface) = self.surfaces.get(&surface_id) else {
            return err(call.tool, vec![format!("surface not found: {surface_id}")]);
        };
        let messages = surface.messages(&surface_id);
        let mut observation = ok(call.tool, "surface preview ready")
            .with_surface_id(surface_id)
            .with_messages(messages.clone());
        if let Err(error) = validate_messages(&messages) {
            observation.ok = false;
            observation.errors.push(error);
        }
        observation
    }

    fn surface_clear(&mut self, call: UiToolCall) -> UiToolObservation {
        let Some(surface_id) = string_arg(&call.args, "surfaceId") else {
            return err(call.tool, vec!["surfaceId is required".to_owned()]);
        };
        self.surfaces.remove(&surface_id);
        ok(call.tool, "surface cleared").with_surface_id(surface_id)
    }

    fn alert(&mut self, call: UiToolCall) -> UiToolObservation {
        let Some(surface_id) = string_arg(&call.args, "surfaceId") else {
            return err(call.tool, vec!["surfaceId is required".to_owned()]);
        };
        let Some(message) = string_arg(&call.args, "message") else {
            return err(call.tool, vec!["message is required".to_owned()]);
        };
        let id = string_arg(&call.args, "id").unwrap_or_else(|| "alert".to_owned());
        let surface = self
            .surfaces
            .entry(surface_id.clone())
            .or_insert_with(|| UiToolSurface {
                catalog_id: A2UI_BASIC_CATALOG_ID.to_owned(),
                recipe: None,
                components: BTreeMap::new(),
            });

        let variant = string_arg(&call.args, "variant").unwrap_or_else(|| "soft".to_owned());
        let tone = string_arg(&call.args, "tone").unwrap_or_else(|| "info".to_owned());
        surface.recipe = Some(
            UiToolRecipe::new(
                "alert",
                variant,
                "https://preline.co/docs/components/alerts.html#discovery",
            )
            .with_tone(tone),
        );
        let row_id = format!("{id}-row");
        let icon_id = format!("{id}-icon");
        let text_id = format!("{id}-text");
        surface.components.insert(
            "root".to_owned(),
            BasicComponent::Card(Card {
                id: "root".to_owned(),
                child: row_id.clone(),
            }),
        );
        surface.components.insert(
            row_id.clone(),
            BasicComponent::Row(Row {
                id: row_id.clone(),
                children: ChildList::Static(vec![icon_id.clone(), text_id.clone()]),
                justify: None,
                align: Some(Align::Center),
            }),
        );
        surface.components.insert(
            icon_id.clone(),
            BasicComponent::Icon(Icon {
                id: icon_id,
                name: IconName::Known(KnownIcon::Warning),
            }),
        );
        surface.components.insert(
            text_id.clone(),
            BasicComponent::Text(Text {
                id: text_id,
                text: DynamicString::Literal(message),
                variant: Some(TextVariant::Body),
            }),
        );

        ok(call.tool, "alert lowered to A2UI Basic components")
            .with_surface_id(surface_id)
            .with_component_id(id)
    }

    fn card(&mut self, call: UiToolCall) -> UiToolObservation {
        let Some(surface_id) = string_arg(&call.args, "surfaceId") else {
            return err(call.tool, vec!["surfaceId is required".to_owned()]);
        };
        let Some(title) = string_arg(&call.args, "title") else {
            return err(call.tool, vec!["title is required".to_owned()]);
        };
        let Some(description) = string_arg(&call.args, "description") else {
            return err(call.tool, vec!["description is required".to_owned()]);
        };
        let id = string_arg(&call.args, "id").unwrap_or_else(|| "draft-card".to_owned());
        let surface = self
            .surfaces
            .entry(surface_id.clone())
            .or_insert_with(|| UiToolSurface {
                catalog_id: A2UI_BASIC_CATALOG_ID.to_owned(),
                recipe: None,
                components: BTreeMap::new(),
            });

        let link_label = string_arg(&call.args, "linkLabel");
        let link_href = string_arg(&call.args, "linkHref");
        surface.recipe = Some(
            UiToolRecipe::new(
                "card",
                "simple",
                "https://preline.co/docs/components/card.html#simple-card",
            )
            .with_optional_link(link_label, link_href),
        );

        let body_id = format!("{id}-body");
        let title_id = format!("{id}-title");
        let description_id = format!("{id}-description");
        surface
            .components
            .insert("root".to_owned(), card_component("root", &body_id));
        surface.components.insert(
            body_id.clone(),
            column_component(&body_id, [title_id.clone(), description_id.clone()]),
        );
        surface.components.insert(
            title_id.clone(),
            text_component(&title_id, title, Some(TextVariant::H3)),
        );
        surface.components.insert(
            description_id.clone(),
            text_component(&description_id, description, Some(TextVariant::Body)),
        );

        ok(call.tool, "card lowered to A2UI Basic components")
            .with_surface_id(surface_id)
            .with_component_id(id)
    }

    fn table(&mut self, call: UiToolCall) -> UiToolObservation {
        let Some(surface_id) = string_arg(&call.args, "surfaceId") else {
            return err(call.tool, vec!["surfaceId is required".to_owned()]);
        };
        let id = string_arg(&call.args, "id").unwrap_or_else(|| "draft-table".to_owned());
        let title = string_arg(&call.args, "title");
        let columns = match string_array_arg(&call.args, "columns") {
            Ok(columns) => columns,
            Err(error) => return err(call.tool, vec![error]),
        };
        if columns.is_empty() {
            return err(call.tool, vec!["columns must not be empty".to_owned()]);
        }
        let rows = match string_matrix_arg(&call.args, "rows") {
            Ok(rows) => rows,
            Err(error) => return err(call.tool, vec![error]),
        };
        for (index, row) in rows.iter().enumerate() {
            if row.len() != columns.len() {
                return err(
                    call.tool,
                    vec![format!(
                        "row {index} has {} cells but columns has {}",
                        row.len(),
                        columns.len()
                    )],
                );
            }
        }

        let surface = self
            .surfaces
            .entry(surface_id.clone())
            .or_insert_with(|| UiToolSurface {
                catalog_id: A2UI_BASIC_CATALOG_ID.to_owned(),
                recipe: None,
                components: BTreeMap::new(),
            });

        surface.recipe = Some(UiToolRecipe::new(
            "table",
            "basic",
            "https://preline.co/docs/components/tables.html#basic-table",
        ));

        let table_id = format!("{id}-table");
        let header_id = format!("{id}-header");
        let mut table_children = Vec::new();
        if let Some(title) = title {
            let title_id = format!("{id}-title");
            surface.components.insert(
                title_id.clone(),
                text_component(&title_id, title, Some(TextVariant::H3)),
            );
            table_children.push(title_id);
        }
        table_children.push(header_id.clone());

        let header_cells: Vec<String> = columns
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let cell_id = format!("{id}-header-cell-{index}");
                surface.components.insert(
                    cell_id.clone(),
                    text_component(&cell_id, value, Some(TextVariant::Caption)),
                );
                cell_id
            })
            .collect();
        surface.components.insert(
            header_id.clone(),
            row_component_vec(&header_id, header_cells, Some(Align::Center)),
        );

        for (row_index, row) in rows.into_iter().enumerate() {
            let row_id = format!("{id}-row-{row_index}");
            let mut cell_ids = Vec::new();
            for (cell_index, value) in row.into_iter().enumerate() {
                let cell_id = format!("{id}-row-{row_index}-cell-{cell_index}");
                surface.components.insert(
                    cell_id.clone(),
                    text_component(&cell_id, value, Some(TextVariant::Body)),
                );
                cell_ids.push(cell_id);
            }
            surface.components.insert(
                row_id.clone(),
                row_component_vec(&row_id, cell_ids, Some(Align::Center)),
            );
            table_children.push(row_id);
        }

        surface
            .components
            .insert("root".to_owned(), card_component("root", table_id.clone()));
        surface.components.insert(
            table_id.clone(),
            column_component_vec(&table_id, table_children),
        );

        ok(call.tool, "table lowered to A2UI Basic components")
            .with_surface_id(surface_id)
            .with_component_id(id)
    }

    fn ask(&mut self, call: UiToolCall) -> UiToolObservation {
        let Some(surface_id) = string_arg(&call.args, "surfaceId") else {
            return err(call.tool, vec!["surfaceId is required".to_owned()]);
        };
        let Some(text) = string_arg(&call.args, "text") else {
            return err(call.tool, vec!["text is required".to_owned()]);
        };
        let yes = string_arg(&call.args, "yes").unwrap_or_else(|| "Yes".to_owned());
        let no = string_arg(&call.args, "no").unwrap_or_else(|| "No".to_owned());
        let id = string_arg(&call.args, "id").unwrap_or_else(|| "draft-ask".to_owned());
        let surface = self
            .surfaces
            .entry(surface_id.clone())
            .or_insert_with(|| UiToolSurface {
                catalog_id: A2UI_BASIC_CATALOG_ID.to_owned(),
                recipe: None,
                components: BTreeMap::new(),
            });

        surface.recipe = Some(UiToolRecipe::new(
            "modal",
            "basic",
            "https://preline.co/docs/components/modal.html",
        ));

        let title_id = format!("{id}-title");
        let modal_id = format!("{id}-modal");
        let open_button_id = format!("{id}-open");
        let open_text_id = format!("{id}-open-text");
        let content_id = format!("{id}-content");
        let message_id = format!("{id}-message");
        let actions_id = format!("{id}-actions");
        let no_button_id = format!("{id}-no");
        let no_text_id = format!("{id}-no-text");
        let yes_button_id = format!("{id}-yes");
        let yes_text_id = format!("{id}-yes-text");

        surface.components.insert(
            "root".to_owned(),
            column_component("root", [title_id.clone(), modal_id.clone()]),
        );
        surface.components.insert(
            title_id.clone(),
            text_component(&title_id, "Question", Some(TextVariant::H3)),
        );
        surface.components.insert(
            modal_id.clone(),
            BasicComponent::Modal(Modal {
                id: modal_id,
                trigger: open_button_id.clone(),
                content: content_id.clone(),
            }),
        );
        surface.components.insert(
            open_button_id.clone(),
            button_component(
                &open_button_id,
                &open_text_id,
                format!("{id}.open"),
                ButtonVariant::Primary,
            ),
        );
        surface.components.insert(
            open_text_id.clone(),
            text_component(&open_text_id, "Open question", None),
        );
        surface.components.insert(
            content_id.clone(),
            column_component(&content_id, [message_id.clone(), actions_id.clone()]),
        );
        surface.components.insert(
            message_id.clone(),
            text_component(&message_id, text, Some(TextVariant::Body)),
        );
        surface.components.insert(
            actions_id.clone(),
            row_component(
                &actions_id,
                [no_button_id.clone(), yes_button_id.clone()],
                Some(Align::Center),
            ),
        );
        surface.components.insert(
            no_button_id.clone(),
            button_component(
                &no_button_id,
                &no_text_id,
                format!("{id}.no"),
                ButtonVariant::Default,
            ),
        );
        surface
            .components
            .insert(no_text_id.clone(), text_component(&no_text_id, no, None));
        surface.components.insert(
            yes_button_id.clone(),
            button_component(
                &yes_button_id,
                &yes_text_id,
                format!("{id}.yes"),
                ButtonVariant::Primary,
            ),
        );
        surface
            .components
            .insert(yes_text_id.clone(), text_component(&yes_text_id, yes, None));

        ok(call.tool, "ask modal lowered to A2UI Basic components")
            .with_surface_id(surface_id)
            .with_component_id(id)
    }
}

impl UiToolRecipe {
    fn new(
        component: impl Into<String>,
        variant: impl Into<String>,
        docs_url: impl Into<String>,
    ) -> Self {
        Self {
            component: component.into(),
            variant: variant.into(),
            docs_url: docs_url.into(),
            tone: None,
            link: None,
        }
    }

    fn with_tone(mut self, tone: impl Into<String>) -> Self {
        self.tone = Some(tone.into());
        self
    }

    fn with_optional_link(mut self, label: Option<String>, href: Option<String>) -> Self {
        if let (Some(label), Some(href)) = (label, href) {
            self.link = Some(UiToolLink { label, href });
        }
        self
    }
}

impl UiToolSurface {
    fn messages(&self, surface_id: &str) -> Vec<A2uiServerMessage> {
        vec![
            A2uiServerMessage::CreateSurface {
                version: A2uiVersion::V0_9,
                create_surface: CreateSurface {
                    surface_id: surface_id.to_owned(),
                    catalog_id: A2UI_BASIC_CATALOG_ID.to_owned(),
                    send_data_model: Some(true),
                    theme: None,
                },
            },
            A2uiServerMessage::UpdateComponents {
                version: A2uiVersion::V0_9,
                update_components: UpdateComponents {
                    surface_id: surface_id.to_owned(),
                    components: self.components.values().cloned().collect(),
                },
            },
        ]
    }
}

impl UiToolObservation {
    fn with_surface_id(mut self, surface_id: impl Into<String>) -> Self {
        self.surface_id = Some(surface_id.into());
        self
    }

    fn with_component_id(mut self, component_id: impl Into<String>) -> Self {
        self.component_id = Some(component_id.into());
        self
    }

    fn with_messages(mut self, messages: Vec<A2uiServerMessage>) -> Self {
        self.messages = messages;
        self
    }
}

pub fn run_tool_program(program: UiToolProgram) -> UiToolProgramResult {
    UiToolRunner::default().run(program)
}

pub fn typed_ui_recipe_components() -> &'static [&'static str] {
    TYPED_UI_RECIPE_COMPONENTS
}

pub fn acceptance_program() -> UiToolProgram {
    UiToolProgram {
        calls: vec![
            UiToolCall {
                tool: "ui.surface.create".to_owned(),
                args: json!({
                    "id": "tool-acceptance",
                    "kind": "sidePanel",
                    "catalog": A2UI_BASIC_CATALOG_ID,
                }),
            },
            UiToolCall {
                tool: "ui.alert".to_owned(),
                args: json!({
                    "surfaceId": "tool-acceptance",
                    "id": "review-required",
                    "message": "Security review required",
                    "variant": "soft",
                    "tone": "warning"
                }),
            },
            UiToolCall {
                tool: "ui.surface.validate".to_owned(),
                args: json!({ "surfaceId": "tool-acceptance" }),
            },
            UiToolCall {
                tool: "ui.surface.preview".to_owned(),
                args: json!({ "surfaceId": "tool-acceptance" }),
            },
        ],
    }
}

pub fn check_template(spec: &TemplateSpec, html: &str) -> TemplateCheckReport {
    let mut errors = Vec::new();
    if spec.schema != "capsem.ui-template.v1" {
        errors.push(format!("unsupported template schema: {}", spec.schema));
    }

    let Some(contract) = component_contract(&spec.component) else {
        errors.push(format!("unknown template component: {}", spec.component));
        return TemplateCheckReport {
            ok: false,
            component: spec.component.clone(),
            variant: spec.variant.clone(),
            errors,
        };
    };

    if !contract
        .variants
        .iter()
        .any(|variant| *variant == spec.variant.as_str())
    {
        errors.push(format!(
            "variant {} is not allowed for {}",
            spec.variant, spec.component
        ));
    }

    let structural: BTreeSet<&str> = spec.structural.iter().map(String::as_str).collect();
    let bound: BTreeSet<&str> = spec
        .bindings
        .iter()
        .map(|binding| binding.prop.as_str())
        .collect();

    for binding in &spec.bindings {
        if !contract
            .props
            .iter()
            .any(|prop| *prop == binding.prop.as_str())
        {
            errors.push(format!("unknown binding prop: {}", binding.prop));
        }
        if !selector_exists(html, &binding.selector) {
            errors.push(format!("selector not found: {}", binding.selector));
        }
    }

    for required in contract.required {
        if !bound.contains(required) && !structural.contains(required) {
            errors.push(format!(
                "required prop is not bound or structural: {required}"
            ));
        }
    }

    for marker in find_capui_markers(html) {
        if !spec
            .bindings
            .iter()
            .any(|binding| selector_matches_marker(&binding.selector, &marker))
        {
            errors.push(format!("unknown template marker: {marker}"));
        }
    }

    TemplateCheckReport {
        ok: errors.is_empty(),
        component: spec.component.clone(),
        variant: spec.variant.clone(),
        errors,
    }
}

struct ComponentContract {
    props: &'static [&'static str],
    required: &'static [&'static str],
    variants: &'static [&'static str],
}

fn component_contract(component: &str) -> Option<ComponentContract> {
    match component {
        "Alert" => Some(ComponentContract {
            props: &["id", "tone", "title", "message"],
            required: &["message"],
            variants: &["soft", "solid", "bordered", "dismissible"],
        }),
        "StatusCard" => Some(ComponentContract {
            props: &["id", "tone", "title", "message"],
            required: &["title", "message"],
            variants: &["top-border"],
        }),
        "Card" => Some(ComponentContract {
            props: &["id", "child"],
            required: &["id", "child"],
            variants: &["simple", "top-border"],
        }),
        "Button" => Some(ComponentContract {
            props: &["id", "child", "action", "variant"],
            required: &["id", "child", "action"],
            variants: &["primary", "default", "borderless"],
        }),
        "Modal" => Some(ComponentContract {
            props: &["id", "trigger", "content"],
            required: &["id", "trigger", "content"],
            variants: &["basic"],
        }),
        _ => None,
    }
}

fn selector_exists(html: &str, selector: &str) -> bool {
    let Some(marker) = selector_marker(selector) else {
        return html.contains(selector);
    };
    html.contains(&marker) || html.contains(&marker.replace('"', "'"))
}

fn selector_marker(selector: &str) -> Option<String> {
    let start = selector.find("[data-capui-")?;
    let slice = &selector[start + 1..selector.len().saturating_sub(1)];
    let mut parts = slice.split('=');
    let attr = parts.next()?;
    let value = parts.next()?.trim_matches('"').trim_matches('\'');
    Some(format!("{attr}=\"{value}"))
}

fn selector_matches_marker(selector: &str, marker: &str) -> bool {
    let Some((attr, value)) = marker.split_once("=\"") else {
        return false;
    };
    let value = value.trim_end_matches('"');
    selector.contains(attr)
        && (selector.contains(&format!("'{value}'")) || selector.contains(&format!("\"{value}\"")))
}

fn find_capui_markers(html: &str) -> BTreeSet<String> {
    let mut markers = BTreeSet::new();
    for attr in [
        "data-capui-text",
        "data-capui-slot",
        "data-capui-action",
        "data-capui-icon",
        "data-capui-state",
    ] {
        let pattern = format!("{attr}=");
        let mut rest = html;
        while let Some(index) = rest.find(&pattern) {
            rest = &rest[index + pattern.len()..];
            let Some(quote) = rest.chars().next() else {
                break;
            };
            if quote != '"' && quote != '\'' {
                continue;
            }
            rest = &rest[1..];
            if let Some(end) = rest.find(quote) {
                markers.insert(format!("{attr}=\"{}\"", &rest[..end]));
                rest = &rest[end + 1..];
            } else {
                break;
            }
        }
    }
    markers
}

fn ok(tool: impl Into<String>, message: impl Into<String>) -> UiToolObservation {
    UiToolObservation {
        tool: tool.into(),
        ok: true,
        message: Some(message.into()),
        surface_id: None,
        component_id: None,
        errors: Vec::new(),
        messages: Vec::new(),
    }
}

fn err(tool: impl Into<String>, errors: Vec<String>) -> UiToolObservation {
    UiToolObservation {
        tool: tool.into(),
        ok: false,
        message: None,
        surface_id: None,
        component_id: None,
        errors,
        messages: Vec::new(),
    }
}

fn string_arg(args: &Value, name: &str) -> Option<String> {
    args.get(name)?.as_str().map(ToOwned::to_owned)
}

fn string_array_arg(args: &Value, name: &str) -> Result<Vec<String>, String> {
    let Some(value) = args.get(name) else {
        return Err(format!("{name} is required"));
    };
    let Some(items) = value.as_array() else {
        return Err(format!("{name} must be an array of strings"));
    };
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| format!("{name} must be an array of strings"))
        })
        .collect()
}

fn string_matrix_arg(args: &Value, name: &str) -> Result<Vec<Vec<String>>, String> {
    let Some(value) = args.get(name) else {
        return Err(format!("{name} is required"));
    };
    let Some(rows) = value.as_array() else {
        return Err(format!("{name} must be an array of string arrays"));
    };
    rows.iter()
        .map(|row| {
            let Some(cells) = row.as_array() else {
                return Err(format!("{name} must be an array of string arrays"));
            };
            cells
                .iter()
                .map(|cell| {
                    cell.as_str()
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| format!("{name} must be an array of string arrays"))
                })
                .collect()
        })
        .collect()
}

fn contains_forbidden_renderer_input(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            matches!(
                key.as_str(),
                "html" | "innerHTML" | "class" | "className" | "style" | "script"
            ) || contains_forbidden_renderer_input(value)
        }),
        Value::Array(items) => items.iter().any(contains_forbidden_renderer_input),
        _ => false,
    }
}

fn text_component(
    id: impl Into<String>,
    value: impl Into<String>,
    variant: Option<TextVariant>,
) -> BasicComponent {
    BasicComponent::Text(Text {
        id: id.into(),
        text: DynamicString::Literal(value.into()),
        variant,
    })
}

fn card_component(id: impl Into<String>, child: impl Into<String>) -> BasicComponent {
    BasicComponent::Card(Card {
        id: id.into(),
        child: child.into(),
    })
}

fn row_component<const N: usize>(
    id: impl Into<String>,
    children: [String; N],
    align: Option<Align>,
) -> BasicComponent {
    BasicComponent::Row(Row {
        id: id.into(),
        children: ChildList::Static(children.into_iter().collect()),
        justify: None,
        align,
    })
}

fn column_component<const N: usize>(
    id: impl Into<String>,
    children: [String; N],
) -> BasicComponent {
    BasicComponent::Column(Column {
        id: id.into(),
        children: ChildList::Static(children.into_iter().collect()),
        justify: None,
        align: None,
    })
}

fn row_component_vec(
    id: impl Into<String>,
    children: Vec<String>,
    align: Option<Align>,
) -> BasicComponent {
    BasicComponent::Row(Row {
        id: id.into(),
        children: ChildList::Static(children),
        justify: None,
        align,
    })
}

fn column_component_vec(id: impl Into<String>, children: Vec<String>) -> BasicComponent {
    BasicComponent::Column(Column {
        id: id.into(),
        children: ChildList::Static(children),
        justify: None,
        align: None,
    })
}

fn button_component(
    id: impl Into<String>,
    child: impl Into<String>,
    action_name: impl Into<String>,
    variant: ButtonVariant,
) -> BasicComponent {
    BasicComponent::Button(Button {
        id: id.into(),
        child: child.into(),
        variant: Some(variant),
        action: Action::Event {
            event: EventAction {
                name: action_name.into(),
                context: BTreeMap::new(),
            },
        },
    })
}
