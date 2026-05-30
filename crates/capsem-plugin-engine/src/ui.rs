use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const A2UI_VERSION: &str = "v0.9";
pub const A2UI_BASIC_CATALOG_ID: &str =
    "https://a2ui.org/specification/v0_9/catalogs/basic/catalog.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum A2uiServerMessage {
    CreateSurface {
        version: A2uiVersion,
        #[serde(rename = "createSurface")]
        create_surface: CreateSurface,
    },
    UpdateComponents {
        version: A2uiVersion,
        #[serde(rename = "updateComponents")]
        update_components: UpdateComponents,
    },
    UpdateDataModel {
        version: A2uiVersion,
        #[serde(rename = "updateDataModel")]
        update_data_model: UpdateDataModel,
    },
    DeleteSurface {
        version: A2uiVersion,
        #[serde(rename = "deleteSurface")]
        delete_surface: DeleteSurface,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum A2uiVersion {
    #[serde(rename = "v0.9")]
    V0_9,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateSurface {
    pub surface_id: String,
    pub catalog_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_data_model: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateComponents {
    pub surface_id: String,
    pub components: Vec<BasicComponent>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateDataModel {
    pub surface_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteSurface {
    pub surface_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "component")]
pub enum BasicComponent {
    Text(Text),
    Image(Image),
    Icon(Icon),
    Video(Video),
    AudioPlayer(AudioPlayer),
    Row(Row),
    Column(Column),
    List(List),
    Card(Card),
    Tabs(Tabs),
    Modal(Modal),
    Divider(Divider),
    Button(Button),
    TextField(TextField),
    CheckBox(CheckBox),
    ChoicePicker(ChoicePicker),
    Slider(Slider),
    DateTimeInput(DateTimeInput),
}

impl BasicComponent {
    pub fn id(&self) -> &str {
        match self {
            Self::Text(component) => &component.id,
            Self::Image(component) => &component.id,
            Self::Icon(component) => &component.id,
            Self::Video(component) => &component.id,
            Self::AudioPlayer(component) => &component.id,
            Self::Row(component) => &component.id,
            Self::Column(component) => &component.id,
            Self::List(component) => &component.id,
            Self::Card(component) => &component.id,
            Self::Tabs(component) => &component.id,
            Self::Modal(component) => &component.id,
            Self::Divider(component) => &component.id,
            Self::Button(component) => &component.id,
            Self::TextField(component) => &component.id,
            Self::CheckBox(component) => &component.id,
            Self::ChoicePicker(component) => &component.id,
            Self::Slider(component) => &component.id,
            Self::DateTimeInput(component) => &component.id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Text {
    pub id: String,
    pub text: DynamicString,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<TextVariant>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextVariant {
    H1,
    H2,
    H3,
    H4,
    H5,
    Caption,
    Body,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub id: String,
    pub url: DynamicString,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<DynamicString>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<ImageFit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<ImageVariant>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ImageFit {
    #[serde(rename = "contain")]
    Contain,
    #[serde(rename = "cover")]
    Cover,
    #[serde(rename = "fill")]
    Fill,
    #[serde(rename = "none")]
    None,
    #[serde(rename = "scaleDown")]
    ScaleDown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ImageVariant {
    #[serde(rename = "icon")]
    Icon,
    #[serde(rename = "avatar")]
    Avatar,
    #[serde(rename = "smallFeature")]
    SmallFeature,
    #[serde(rename = "mediumFeature")]
    MediumFeature,
    #[serde(rename = "largeFeature")]
    LargeFeature,
    #[serde(rename = "header")]
    Header,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Icon {
    pub id: String,
    pub name: IconName,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IconName {
    Known(KnownIcon),
    SvgPath {
        #[serde(rename = "svgPath")]
        svg_path: String,
    },
    Binding(DataBinding),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum KnownIcon {
    #[serde(rename = "accountCircle")]
    AccountCircle,
    #[serde(rename = "add")]
    Add,
    #[serde(rename = "arrowBack")]
    ArrowBack,
    #[serde(rename = "arrowForward")]
    ArrowForward,
    #[serde(rename = "attachFile")]
    AttachFile,
    #[serde(rename = "calendarToday")]
    CalendarToday,
    #[serde(rename = "call")]
    Call,
    #[serde(rename = "camera")]
    Camera,
    #[serde(rename = "check")]
    Check,
    #[serde(rename = "close")]
    Close,
    #[serde(rename = "delete")]
    Delete,
    #[serde(rename = "download")]
    Download,
    #[serde(rename = "edit")]
    Edit,
    #[serde(rename = "event")]
    Event,
    #[serde(rename = "error")]
    Error,
    #[serde(rename = "fastForward")]
    FastForward,
    #[serde(rename = "favorite")]
    Favorite,
    #[serde(rename = "favoriteOff")]
    FavoriteOff,
    #[serde(rename = "folder")]
    Folder,
    #[serde(rename = "help")]
    Help,
    #[serde(rename = "home")]
    Home,
    #[serde(rename = "info")]
    Info,
    #[serde(rename = "locationOn")]
    LocationOn,
    #[serde(rename = "lock")]
    Lock,
    #[serde(rename = "lockOpen")]
    LockOpen,
    #[serde(rename = "mail")]
    Mail,
    #[serde(rename = "menu")]
    Menu,
    #[serde(rename = "moreVert")]
    MoreVert,
    #[serde(rename = "moreHoriz")]
    MoreHoriz,
    #[serde(rename = "notificationsOff")]
    NotificationsOff,
    #[serde(rename = "notifications")]
    Notifications,
    #[serde(rename = "pause")]
    Pause,
    #[serde(rename = "payment")]
    Payment,
    #[serde(rename = "person")]
    Person,
    #[serde(rename = "phone")]
    Phone,
    #[serde(rename = "photo")]
    Photo,
    #[serde(rename = "play")]
    Play,
    #[serde(rename = "print")]
    Print,
    #[serde(rename = "refresh")]
    Refresh,
    #[serde(rename = "rewind")]
    Rewind,
    #[serde(rename = "search")]
    Search,
    #[serde(rename = "send")]
    Send,
    #[serde(rename = "settings")]
    Settings,
    #[serde(rename = "share")]
    Share,
    #[serde(rename = "shoppingCart")]
    ShoppingCart,
    #[serde(rename = "skipNext")]
    SkipNext,
    #[serde(rename = "skipPrevious")]
    SkipPrevious,
    #[serde(rename = "star")]
    Star,
    #[serde(rename = "starHalf")]
    StarHalf,
    #[serde(rename = "starOff")]
    StarOff,
    #[serde(rename = "stop")]
    Stop,
    #[serde(rename = "upload")]
    Upload,
    #[serde(rename = "visibility")]
    Visibility,
    #[serde(rename = "visibilityOff")]
    VisibilityOff,
    #[serde(rename = "volumeDown")]
    VolumeDown,
    #[serde(rename = "volumeMute")]
    VolumeMute,
    #[serde(rename = "volumeOff")]
    VolumeOff,
    #[serde(rename = "volumeUp")]
    VolumeUp,
    #[serde(rename = "warning")]
    Warning,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Video {
    pub id: String,
    pub url: DynamicString,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioPlayer {
    pub id: String,
    pub url: DynamicString,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<DynamicString>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub id: String,
    pub children: ChildList,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justify: Option<Justify>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub id: String,
    pub children: ChildList,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justify: Option<Justify>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Justify {
    #[serde(rename = "start")]
    Start,
    #[serde(rename = "center")]
    Center,
    #[serde(rename = "end")]
    End,
    #[serde(rename = "spaceBetween")]
    SpaceBetween,
    #[serde(rename = "spaceAround")]
    SpaceAround,
    #[serde(rename = "spaceEvenly")]
    SpaceEvenly,
    #[serde(rename = "stretch")]
    Stretch,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Align {
    #[serde(rename = "start")]
    Start,
    #[serde(rename = "center")]
    Center,
    #[serde(rename = "end")]
    End,
    #[serde(rename = "stretch")]
    Stretch,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct List {
    pub id: String,
    pub children: ChildList,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<ListDirection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ListDirection {
    #[serde(rename = "vertical")]
    Vertical,
    #[serde(rename = "horizontal")]
    Horizontal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Card {
    pub id: String,
    pub child: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tabs {
    pub id: String,
    pub tabs: Vec<Tab>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tab {
    pub title: DynamicString,
    pub child: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Modal {
    pub id: String,
    pub trigger: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Divider {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis: Option<Axis>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Axis {
    #[serde(rename = "horizontal")]
    Horizontal,
    #[serde(rename = "vertical")]
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Button {
    pub id: String,
    pub child: String,
    pub action: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<ButtonVariant>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ButtonVariant {
    #[serde(rename = "default")]
    Default,
    #[serde(rename = "primary")]
    Primary,
    #[serde(rename = "borderless")]
    Borderless,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextField {
    pub id: String,
    pub label: DynamicString,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<DynamicString>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<TextFieldVariant>,
    #[serde(rename = "validationRegexp", skip_serializing_if = "Option::is_none")]
    pub validation_regexp: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TextFieldVariant {
    #[serde(rename = "longText")]
    LongText,
    #[serde(rename = "number")]
    Number,
    #[serde(rename = "shortText")]
    ShortText,
    #[serde(rename = "obscured")]
    Obscured,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckBox {
    pub id: String,
    pub label: DynamicString,
    pub value: DynamicBoolean,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoicePicker {
    pub id: String,
    pub options: Vec<ChoiceOption>,
    pub value: DynamicStringList,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<DynamicString>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<ChoicePickerVariant>,
    #[serde(rename = "displayStyle", skip_serializing_if = "Option::is_none")]
    pub display_style: Option<ChoiceDisplayStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filterable: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceOption {
    pub label: DynamicString,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ChoicePickerVariant {
    #[serde(rename = "multipleSelection")]
    MultipleSelection,
    #[serde(rename = "mutuallyExclusive")]
    MutuallyExclusive,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ChoiceDisplayStyle {
    #[serde(rename = "checkbox")]
    Checkbox,
    #[serde(rename = "chips")]
    Chips,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slider {
    pub id: String,
    pub value: DynamicNumber,
    pub max: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<DynamicString>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DateTimeInput {
    pub id: String,
    pub value: DynamicString,
    #[serde(rename = "enableDate", skip_serializing_if = "Option::is_none")]
    pub enable_date: Option<bool>,
    #[serde(rename = "enableTime", skip_serializing_if = "Option::is_none")]
    pub enable_time: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<DynamicString>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<DynamicString>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<DynamicString>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChildList {
    Static(Vec<String>),
    Template {
        #[serde(rename = "componentId")]
        component_id: String,
        path: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DynamicString {
    Literal(String),
    Binding(DataBinding),
    Function(FunctionCall),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DynamicNumber {
    Literal(f64),
    Binding(DataBinding),
    Function(FunctionCall),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DynamicBoolean {
    Literal(bool),
    Binding(DataBinding),
    Function(FunctionCall),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DynamicStringList {
    Literal(Vec<String>),
    Binding(DataBinding),
    Function(FunctionCall),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DynamicValue {
    String(String),
    Number(f64),
    Boolean(bool),
    Array(Vec<Value>),
    Binding(DataBinding),
    Function(FunctionCall),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataBinding {
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FunctionCall {
    pub call: BasicFunction,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, Value>,
    #[serde(rename = "returnType", skip_serializing_if = "Option::is_none")]
    pub return_type: Option<ReturnType>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BasicFunction {
    #[serde(rename = "required")]
    Required,
    #[serde(rename = "regex")]
    Regex,
    #[serde(rename = "length")]
    Length,
    #[serde(rename = "numeric")]
    Numeric,
    #[serde(rename = "email")]
    Email,
    #[serde(rename = "formatString")]
    FormatString,
    #[serde(rename = "formatNumber")]
    FormatNumber,
    #[serde(rename = "formatCurrency")]
    FormatCurrency,
    #[serde(rename = "formatDate")]
    FormatDate,
    #[serde(rename = "pluralize")]
    Pluralize,
    #[serde(rename = "openUrl")]
    OpenUrl,
    #[serde(rename = "and")]
    And,
    #[serde(rename = "or")]
    Or,
    #[serde(rename = "not")]
    Not,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ReturnType {
    #[serde(rename = "string")]
    String,
    #[serde(rename = "number")]
    Number,
    #[serde(rename = "boolean")]
    Boolean,
    #[serde(rename = "array")]
    Array,
    #[serde(rename = "object")]
    Object,
    #[serde(rename = "any")]
    Any,
    #[serde(rename = "void")]
    Void,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Action {
    Event {
        event: EventAction,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: FunctionCall,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventAction {
    pub name: String,
    #[serde(default)]
    pub context: BTreeMap<String, DynamicValue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct A2uiDocument {
    pub messages: Vec<A2uiServerMessage>,
}

impl A2uiDocument {
    pub fn new(surface_id: impl Into<String>) -> Self {
        let surface_id = surface_id.into();
        Self {
            messages: vec![A2uiServerMessage::CreateSurface {
                version: A2uiVersion::V0_9,
                create_surface: CreateSurface {
                    surface_id,
                    catalog_id: A2UI_BASIC_CATALOG_ID.to_owned(),
                    send_data_model: Some(true),
                    theme: None,
                },
            }],
        }
    }

    pub fn components(
        mut self,
        surface_id: impl Into<String>,
        components: Vec<BasicComponent>,
    ) -> Self {
        self.messages.push(A2uiServerMessage::UpdateComponents {
            version: A2uiVersion::V0_9,
            update_components: UpdateComponents {
                surface_id: surface_id.into(),
                components,
            },
        });
        self
    }

    pub fn data(mut self, surface_id: impl Into<String>, value: Value) -> Self {
        self.messages.push(A2uiServerMessage::UpdateDataModel {
            version: A2uiVersion::V0_9,
            update_data_model: UpdateDataModel {
                surface_id: surface_id.into(),
                path: None,
                value: Some(value),
            },
        });
        self
    }
}

pub struct Ui;

impl Ui {
    pub fn alert(surface_id: impl Into<String>, msg: impl Into<String>) -> A2uiDocument {
        let surface_id = surface_id.into();
        A2uiDocument::new(surface_id.clone()).components(
            surface_id,
            vec![
                card("root", "alert-row"),
                row(
                    "alert-row",
                    ["alert-icon", "alert-text"],
                    Some(Align::Center),
                ),
                BasicComponent::Icon(Icon {
                    id: "alert-icon".to_owned(),
                    name: IconName::Known(KnownIcon::Warning),
                }),
                text("alert-text", msg, Some(TextVariant::Body)),
            ],
        )
    }

    pub fn ask(
        surface_id: impl Into<String>,
        text_value: impl Into<String>,
        yes: impl Into<String>,
        no: impl Into<String>,
    ) -> A2uiDocument {
        let surface_id = surface_id.into();
        A2uiDocument::new(surface_id.clone()).components(
            surface_id,
            vec![
                column("root", ["ask-title", "ask-modal"]),
                text("ask-title", "Question", Some(TextVariant::H3)),
                BasicComponent::Modal(Modal {
                    id: "ask-modal".to_owned(),
                    trigger: "ask-open".to_owned(),
                    content: "ask-content".to_owned(),
                }),
                button(
                    "ask-open",
                    "ask-open-text",
                    "ask.open",
                    ButtonVariant::Primary,
                ),
                text("ask-open-text", "Open question", None),
                column("ask-content", ["ask-message", "ask-actions"]),
                text("ask-message", text_value, Some(TextVariant::Body)),
                row("ask-actions", ["ask-no", "ask-yes"], Some(Align::Center)),
                button("ask-no", "ask-no-text", "ask.no", ButtonVariant::Default),
                text("ask-no-text", no, None),
                button("ask-yes", "ask-yes-text", "ask.yes", ButtonVariant::Primary),
                text("ask-yes-text", yes, None),
            ],
        )
    }

    pub fn card(
        surface_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> A2uiDocument {
        let surface_id = surface_id.into();
        A2uiDocument::new(surface_id.clone()).components(
            surface_id,
            vec![
                card("root", "card-body"),
                column("card-body", ["card-title", "card-description"]),
                text("card-title", title, Some(TextVariant::H3)),
                text("card-description", description, Some(TextVariant::Body)),
            ],
        )
    }

    pub fn weather_card(surface_id: impl Into<String>) -> A2uiDocument {
        let surface_id = surface_id.into();
        A2uiDocument::new(surface_id.clone())
            .components(
                surface_id.clone(),
                vec![
                    card("root", "main-column"),
                    BasicComponent::Column(Column {
                        id: "main-column".to_owned(),
                        children: ChildList::Static(vec![
                            "temp-row".to_owned(),
                            "location".to_owned(),
                            "description".to_owned(),
                            "forecast-row".to_owned(),
                        ]),
                        justify: None,
                        align: Some(Align::Center),
                    }),
                    BasicComponent::Row(Row {
                        id: "temp-row".to_owned(),
                        children: ChildList::Static(vec![
                            "temp-high".to_owned(),
                            "temp-low".to_owned(),
                        ]),
                        justify: None,
                        align: Some(Align::Start),
                    }),
                    text_function(
                        "temp-high",
                        format_string("${/tempHigh}°"),
                        Some(TextVariant::H1),
                    ),
                    text_function(
                        "temp-low",
                        format_string("${/tempLow}°"),
                        Some(TextVariant::H2),
                    ),
                    text_binding("location", "/location", Some(TextVariant::H3)),
                    text_binding("description", "/description", Some(TextVariant::Caption)),
                    BasicComponent::Row(Row {
                        id: "forecast-row".to_owned(),
                        children: ChildList::Template {
                            path: "/forecast".to_owned(),
                            component_id: "forecast-day-template".to_owned(),
                        },
                        justify: Some(Justify::SpaceAround),
                        align: None,
                    }),
                    BasicComponent::Column(Column {
                        id: "forecast-day-template".to_owned(),
                        children: ChildList::Static(vec![
                            "day-name".to_owned(),
                            "day-icon".to_owned(),
                            "day-temp".to_owned(),
                        ]),
                        justify: None,
                        align: Some(Align::Center),
                    }),
                    text_function(
                        "day-name",
                        FunctionCall {
                            call: BasicFunction::FormatDate,
                            args: BTreeMap::from([
                                ("value".to_owned(), json!({ "path": "date" })),
                                ("format".to_owned(), json!("E")),
                            ]),
                            return_type: Some(ReturnType::String),
                        },
                        Some(TextVariant::Caption),
                    ),
                    text_binding("day-icon", "icon", Some(TextVariant::H3)),
                    text_function(
                        "day-temp",
                        format_string("${temp}°"),
                        Some(TextVariant::Caption),
                    ),
                ],
            )
            .data(
                surface_id,
                json!({
                    "tempHigh": 72,
                    "tempLow": 58,
                    "location": "Austin, TX",
                    "description": "Clear skies with light breeze",
                    "forecast": [
                        { "date": "2025-12-16", "icon": "sunny", "temp": 74 },
                        { "date": "2025-12-17", "icon": "sunny", "temp": 76 },
                        { "date": "2025-12-18", "icon": "partly cloudy", "temp": 71 },
                    ]
                }),
            )
    }

    pub fn status_callout(
        surface_id: impl Into<String>,
        title: impl Into<String>,
        msg: impl Into<String>,
    ) -> A2uiDocument {
        let surface_id = surface_id.into();
        A2uiDocument::new(surface_id.clone()).components(
            surface_id,
            vec![
                card("root", "status-column"),
                column("status-column", ["status-row", "status-message"]),
                row(
                    "status-row",
                    ["status-icon", "status-title"],
                    Some(Align::Center),
                ),
                BasicComponent::Icon(Icon {
                    id: "status-icon".to_owned(),
                    name: IconName::Known(KnownIcon::Info),
                }),
                text("status-title", title, Some(TextVariant::H4)),
                text("status-message", msg, Some(TextVariant::Body)),
            ],
        )
    }

    pub fn chat_baseline(surface_id: impl Into<String>) -> A2uiDocument {
        let surface_id = surface_id.into();
        A2uiDocument::new(surface_id.clone())
            .components(
                surface_id.clone(),
                vec![
                    card("root", "main-column"),
                    column("main-column", ["header", "divider", "messages-list"]),
                    row("header", ["channel-icon", "channel-name"], Some(Align::Center)),
                    BasicComponent::Icon(Icon {
                        id: "channel-icon".to_owned(),
                        name: IconName::Known(KnownIcon::Info),
                    }),
                    text_binding("channel-name", "/channelName", Some(TextVariant::H3)),
                    BasicComponent::Divider(Divider {
                        id: "divider".to_owned(),
                        axis: None,
                    }),
                    BasicComponent::Column(Column {
                        id: "messages-list".to_owned(),
                        children: ChildList::Template {
                            path: "/messages".to_owned(),
                            component_id: "message-template".to_owned(),
                        },
                        justify: None,
                        align: Some(Align::Start),
                    }),
                    row(
                        "message-template",
                        ["msg-avatar", "msg-content"],
                        Some(Align::Start),
                    ),
                    BasicComponent::Image(Image {
                        id: "msg-avatar".to_owned(),
                        url: DynamicString::Binding(DataBinding {
                            path: "avatar".to_owned(),
                        }),
                        description: None,
                        fit: Some(ImageFit::Cover),
                        variant: Some(ImageVariant::Avatar),
                    }),
                    column("msg-content", ["msg-header", "msg-text"]),
                    row(
                        "msg-header",
                        ["msg-username", "msg-time"],
                        Some(Align::Center),
                    ),
                    text_binding("msg-username", "username", Some(TextVariant::H4)),
                    text_function(
                        "msg-time",
                        FunctionCall {
                            call: BasicFunction::FormatDate,
                            args: BTreeMap::from([
                                ("value".to_owned(), json!({ "path": "timestamp" })),
                                ("format".to_owned(), json!("h:mm a")),
                            ]),
                            return_type: Some(ReturnType::String),
                        },
                        Some(TextVariant::Caption),
                    ),
                    text_binding("msg-text", "text", Some(TextVariant::Body)),
                ],
            )
            .data(
                surface_id,
                json!({
                    "channelName": "plugin-preview",
                    "messages": [
                        {
                            "avatar": "https://images.unsplash.com/photo-1472099645785-5658abf4ff4e?w=40&h=40&fit=crop",
                            "username": "Plugin",
                            "timestamp": "2025-12-15T10:32:00Z",
                            "text": "Generated from the same A2UI Basic message path."
                        },
                        {
                            "avatar": "https://images.unsplash.com/photo-1438761681033-6461ffad8d80?w=40&h=40&fit=crop",
                            "username": "Capsem",
                            "timestamp": "2025-12-15T10:45:00Z",
                            "text": "Svelte renders only after Rust validation."
                        }
                    ]
                }),
            )
    }
}

pub fn validate_messages(messages: &[A2uiServerMessage]) -> Result<(), String> {
    let mut surfaces = BTreeMap::<String, String>::new();
    for message in messages {
        validate_against_a2ui_types(message)?;
        match message {
            A2uiServerMessage::CreateSurface { create_surface, .. } => {
                if create_surface.surface_id.trim().is_empty() {
                    return Err("surfaceId must be non-empty".to_owned());
                }
                if create_surface.catalog_id != A2UI_BASIC_CATALOG_ID {
                    return Err("catalogId must be A2UI Basic v0.9".to_owned());
                }
                surfaces.insert(
                    create_surface.surface_id.clone(),
                    create_surface.catalog_id.clone(),
                );
            }
            A2uiServerMessage::UpdateComponents {
                update_components, ..
            } => {
                if !surfaces.contains_key(&update_components.surface_id) {
                    return Err(format!(
                        "surface {} must be created before updateComponents",
                        update_components.surface_id
                    ));
                }
                if update_components.components.is_empty() {
                    return Err("updateComponents requires at least one component".to_owned());
                }
                if !update_components
                    .components
                    .iter()
                    .any(|component| component.id() == "root")
                {
                    return Err("updateComponents requires a root component".to_owned());
                }
            }
            A2uiServerMessage::UpdateDataModel {
                update_data_model, ..
            } => {
                if !surfaces.contains_key(&update_data_model.surface_id) {
                    return Err(format!(
                        "surface {} must be created before updateDataModel",
                        update_data_model.surface_id
                    ));
                }
            }
            A2uiServerMessage::DeleteSurface { delete_surface, .. } => {
                if !surfaces.contains_key(&delete_surface.surface_id) {
                    return Err(format!(
                        "surface {} must be created before deleteSurface",
                        delete_surface.surface_id
                    ));
                }
                surfaces.remove(&delete_surface.surface_id);
            }
        }
    }
    Ok(())
}

fn validate_against_a2ui_types(message: &A2uiServerMessage) -> Result<(), String> {
    let value = serde_json::to_value(message).map_err(|error| error.to_string())?;
    let parsed: a2ui_types::v09::server_to_client::ServerToClientMessage =
        serde_json::from_value(value).map_err(|error| {
            format!("A2UI Rust type validation failed for server message: {error}")
        })?;

    if parsed.version != A2UI_VERSION {
        return Err(format!("version must be {A2UI_VERSION}"));
    }

    let message_count = [
        parsed.create_surface.is_some(),
        parsed.update_components.is_some(),
        parsed.update_data_model.is_some(),
        parsed.delete_surface.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();

    if message_count != 1 {
        return Err("A2UI server message must contain exactly one operation".to_owned());
    }

    Ok(())
}

fn text(
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

fn text_binding(
    id: impl Into<String>,
    path: impl Into<String>,
    variant: Option<TextVariant>,
) -> BasicComponent {
    BasicComponent::Text(Text {
        id: id.into(),
        text: DynamicString::Binding(DataBinding { path: path.into() }),
        variant,
    })
}

fn text_function(
    id: impl Into<String>,
    function: FunctionCall,
    variant: Option<TextVariant>,
) -> BasicComponent {
    BasicComponent::Text(Text {
        id: id.into(),
        text: DynamicString::Function(function),
        variant,
    })
}

fn card(id: impl Into<String>, child: impl Into<String>) -> BasicComponent {
    BasicComponent::Card(Card {
        id: id.into(),
        child: child.into(),
    })
}

fn row<const N: usize>(
    id: impl Into<String>,
    children: [&str; N],
    align: Option<Align>,
) -> BasicComponent {
    BasicComponent::Row(Row {
        id: id.into(),
        children: ChildList::Static(children.into_iter().map(str::to_owned).collect()),
        justify: None,
        align,
    })
}

fn column<const N: usize>(id: impl Into<String>, children: [&str; N]) -> BasicComponent {
    BasicComponent::Column(Column {
        id: id.into(),
        children: ChildList::Static(children.into_iter().map(str::to_owned).collect()),
        justify: None,
        align: None,
    })
}

fn button(
    id: impl Into<String>,
    child: impl Into<String>,
    action_name: impl Into<String>,
    variant: ButtonVariant,
) -> BasicComponent {
    BasicComponent::Button(Button {
        id: id.into(),
        child: child.into(),
        action: Action::Event {
            event: EventAction {
                name: action_name.into(),
                context: BTreeMap::new(),
            },
        },
        variant: Some(variant),
    })
}

fn format_string(value: impl Into<String>) -> FunctionCall {
    FunctionCall {
        call: BasicFunction::FormatString,
        args: BTreeMap::from([("value".to_owned(), json!(value.into()))]),
        return_type: Some(ReturnType::String),
    }
}
