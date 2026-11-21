use super::EnvironmentRevisionReference;
use capsem_config::{
    plugins::{
        EnvironmentPluginSource, PluginControlGroup, PluginDescriptor, PluginDetailCapability, PluginDirection,
        PluginEventFamily, PluginId, PluginStage,
    },
    DetectionLevel, SecurityPluginMode, SettingValue, Widget,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPluginListQuery {
    /// Require the current environment to match this saved revision.
    #[param(format = "uuid")]
    pub revision_id: Option<String>,
}

/// Policy inputs for a new or saved environment under current corporate policy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPluginListResponse {
    /// Null for a new environment. Reading plugins never applies policy to a running sandbox.
    #[schema(required = true)]
    pub revision: Option<EnvironmentRevisionReference>,
    pub plugins: Vec<EnvironmentPluginEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPluginEntry {
    pub descriptor: EnvironmentPluginDescriptor,
    pub source: EnvironmentPluginSource,
    pub locked: bool,
    /// Fields from the saved entry, keyed by descriptor field ID. Empty when inherited.
    pub configured: BTreeMap<String, EnvironmentPluginFieldValue>,
    /// Fields selected by the runtime's whole-entry precedence.
    pub effective: BTreeMap<String, EnvironmentPluginFieldValue>,
    pub directions: Vec<EnvironmentPluginDirectionState>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPluginDirectionState {
    pub direction: PluginDirection,
    pub mode: SecurityPluginMode,
    #[schema(required = true)]
    pub detection_level: Option<DetectionLevel>,
}

/// The descriptor projection used by standard plugin controls. General settings
/// support file and collection values that no built-in plugin field accepts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum EnvironmentPluginFieldValue {
    Toggle(bool),
    Text(String),
}

impl TryFrom<&SettingValue> for EnvironmentPluginFieldValue {
    type Error = String;

    fn try_from(value: &SettingValue) -> Result<Self, Self::Error> {
        match value {
            SettingValue::Bool(value) => Ok(Self::Toggle(*value)),
            SettingValue::Text(value) => Ok(Self::Text(value.clone())),
            _ => Err("unsupported plugin field value".into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPluginDescriptor {
    pub id: PluginId,
    pub version: u16,
    pub name: String,
    pub description: String,
    pub stage: PluginStage,
    pub event_families: Vec<PluginEventFamily>,
    pub directions: Vec<PluginDirection>,
    pub modes: Vec<SecurityPluginMode>,
    pub required: bool,
    pub diagnostic: bool,
    pub groups: Vec<PluginControlGroup>,
    pub fields: Vec<EnvironmentPluginField>,
    pub limitations: Vec<String>,
    pub detail_capabilities: Vec<PluginDetailCapability>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPluginField {
    pub id: String,
    pub category: String,
    pub name: String,
    pub description: String,
    pub default_value: EnvironmentPluginFieldValue,
    pub widget: Option<Widget>,
    pub choices: Vec<String>,
    pub dependencies: Vec<EnvironmentPluginFieldDependency>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPluginFieldDependency {
    pub field: String,
    pub not_equals: EnvironmentPluginFieldValue,
}

impl TryFrom<&PluginDescriptor> for EnvironmentPluginDescriptor {
    type Error = String;

    fn try_from(descriptor: &PluginDescriptor) -> Result<Self, String> {
        let fields = descriptor
            .fields
            .iter()
            .map(|field| {
                let setting = &field.setting;
                let dependencies = field
                    .dependencies
                    .iter()
                    .map(|dependency| {
                        Ok(EnvironmentPluginFieldDependency {
                            field: dependency.field.clone(),
                            not_equals: (&dependency.not_equals).try_into()?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(EnvironmentPluginField {
                    id: setting.id.clone(),
                    category: setting.category.clone(),
                    name: setting.name.clone(),
                    description: setting.description.clone(),
                    default_value: (&setting.default_value).try_into()?,
                    widget: setting.metadata.widget,
                    choices: setting.metadata.choices.clone(),
                    dependencies,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            id: descriptor.id,
            version: descriptor.version,
            name: descriptor.name.into(),
            description: descriptor.description.into(),
            stage: descriptor.stage,
            event_families: descriptor.event_families.clone(),
            directions: descriptor.directions.clone(),
            modes: descriptor.modes.clone(),
            required: descriptor.required,
            diagnostic: descriptor.diagnostic,
            groups: descriptor.groups.clone(),
            fields,
            limitations: descriptor.limitations.iter().copied().map(String::from).collect(),
            detail_capabilities: descriptor.detail_capabilities.clone(),
        })
    }
}
