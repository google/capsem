use capsem_plugin_engine::ui::{
    validate_messages, A2uiServerMessage, A2uiVersion, BasicComponent, CreateSurface,
    A2UI_BASIC_CATALOG_ID,
};
use capsem_plugin_engine::ui::{Ui, UpdateComponents};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct A2uiExample {
    name: String,
    description: String,
    messages: Vec<A2uiServerMessage>,
}

#[test]
fn upstream_weather_current_shape_round_trips() {
    assert_fixture_round_trip(WEATHER_CURRENT);
}

#[test]
fn upstream_modal_shape_round_trips() {
    assert_fixture_round_trip(MODAL_SAMPLE);
}

#[test]
fn upstream_chat_message_shape_round_trips() {
    assert_fixture_round_trip(CHAT_MESSAGE);
}

#[test]
fn typed_helpers_emit_valid_a2ui_basic_messages() {
    for document in [
        Ui::alert("alert-surface", "Security review required"),
        Ui::ask("ask-surface", "Allow this model call?", "Allow", "Deny"),
        Ui::weather_card("weather-surface"),
        Ui::status_callout(
            "status-surface",
            "Baseline catalog",
            "This is composed from A2UI Basic components.",
        ),
        Ui::chat_baseline("chat-surface"),
    ] {
        validate_messages(&document.messages).expect("typed UI helper emits valid A2UI Basic");

        let first = &document.messages[0];
        let A2uiServerMessage::CreateSurface { create_surface, .. } = first else {
            panic!("first message must create the A2UI surface");
        };
        assert_eq!(create_surface.catalog_id, A2UI_BASIC_CATALOG_ID);
    }
}

#[test]
fn rejects_invented_component_names_before_renderer() {
    let payload = json!({
        "version": "v0.9",
        "updateComponents": {
            "surfaceId": "bad-surface",
            "components": [
                {
                    "id": "root",
                    "component": "Alert",
                    "msg": "this is not A2UI Basic"
                }
            ]
        }
    });

    let err = serde_json::from_value::<A2uiServerMessage>(payload).unwrap_err();
    assert!(
        err.to_string().contains("unknown variant")
            || err.to_string().contains("data did not match"),
        "unexpected error: {err}"
    );
}

#[test]
fn rejects_non_basic_catalogs() {
    let messages = vec![A2uiServerMessage::CreateSurface {
        version: A2uiVersion::V0_9,
        create_surface: CreateSurface {
            surface_id: "surface".to_owned(),
            catalog_id: "https://capsem.dev/not-the-baseline/catalog.json".to_owned(),
            send_data_model: None,
            theme: None,
        },
    }];

    let err = validate_messages(&messages).unwrap_err();
    assert_eq!(err, "catalogId must be A2UI Basic v0.9");
}

#[test]
fn rejects_update_before_create() {
    let messages = vec![A2uiServerMessage::UpdateComponents {
        version: A2uiVersion::V0_9,
        update_components: UpdateComponents {
            surface_id: "missing-surface".to_owned(),
            components: vec![BasicComponent::Text(capsem_plugin_engine::ui::Text {
                id: "root".to_owned(),
                text: capsem_plugin_engine::ui::DynamicString::Literal("orphan".to_owned()),
                variant: None,
            })],
        },
    }];

    let err = validate_messages(&messages).unwrap_err();
    assert!(err.contains("must be created before updateComponents"));
}

fn assert_fixture_round_trip(source: &str) {
    let source_value: Value = serde_json::from_str(source).expect("fixture JSON is valid");
    let example: A2uiExample =
        serde_json::from_value(source_value.clone()).expect("fixture parses into Rust A2UI model");

    validate_messages(&example.messages).expect("fixture satisfies Capsem A2UI contract");

    let round_tripped = serde_json::to_value(&example).expect("fixture serializes from Rust model");
    assert_eq!(round_tripped, source_value);
}

const WEATHER_CURRENT: &str = r#"{
  "name": "Weather Current",
  "description": "Example of weather current demonstrating templating and string formatting.",
  "messages": [
    {
      "version": "v0.9",
      "createSurface": {
        "surfaceId": "gallery-weather-current",
        "catalogId": "https://a2ui.org/specification/v0_9/catalogs/basic/catalog.json",
        "sendDataModel": true
      }
    },
    {
      "version": "v0.9",
      "updateComponents": {
        "surfaceId": "gallery-weather-current",
        "components": [
          {
            "id": "root",
            "component": "Card",
            "child": "main-column"
          },
          {
            "id": "main-column",
            "component": "Column",
            "children": ["temp-row", "location", "description", "forecast-row"],
            "align": "center"
          },
          {
            "id": "temp-row",
            "component": "Row",
            "children": ["temp-high", "temp-low"],
            "align": "start"
          },
          {
            "id": "temp-high",
            "component": "Text",
            "text": {
              "call": "formatString",
              "args": {
                "value": "${/tempHigh}\u00b0"
              },
              "returnType": "string"
            },
            "variant": "h1"
          },
          {
            "id": "temp-low",
            "component": "Text",
            "text": {
              "call": "formatString",
              "args": {
                "value": "${/tempLow}\u00b0"
              },
              "returnType": "string"
            },
            "variant": "h2"
          },
          {
            "id": "location",
            "component": "Text",
            "text": {
              "path": "/location"
            },
            "variant": "h3"
          },
          {
            "id": "description",
            "component": "Text",
            "text": {
              "path": "/description"
            },
            "variant": "caption"
          },
          {
            "id": "forecast-row",
            "component": "Row",
            "children": {
              "path": "/forecast",
              "componentId": "forecast-day-template"
            },
            "justify": "spaceAround"
          },
          {
            "id": "forecast-day-template",
            "component": "Column",
            "children": ["day-name", "day-icon", "day-temp"],
            "align": "center"
          },
          {
            "id": "day-name",
            "component": "Text",
            "text": {
              "call": "formatDate",
              "args": {
                "value": {
                  "path": "date"
                },
                "format": "E"
              },
              "returnType": "string"
            },
            "variant": "caption"
          },
          {
            "id": "day-icon",
            "component": "Text",
            "text": {
              "path": "icon"
            },
            "variant": "h3"
          },
          {
            "id": "day-temp",
            "component": "Text",
            "text": {
              "call": "formatString",
              "args": {
                "value": "${temp}\u00b0"
              },
              "returnType": "string"
            },
            "variant": "caption"
          }
        ]
      }
    },
    {
      "version": "v0.9",
      "updateDataModel": {
        "surfaceId": "gallery-weather-current",
        "value": {
          "tempHigh": 72,
          "tempLow": 58,
          "location": "Austin, TX",
          "description": "Clear skies with light breeze",
          "forecast": [
            {
              "date": "2025-12-16",
              "icon": "\u2600\ufe0f",
              "temp": 74
            },
            {
              "date": "2025-12-17",
              "icon": "\u2600\ufe0f",
              "temp": 76
            },
            {
              "date": "2025-12-18",
              "icon": "\u26c5",
              "temp": 71
            }
          ]
        }
      }
    }
  ]
}"#;

const MODAL_SAMPLE: &str = r#"{
  "name": "Modal Sample",
  "description": "Example of Modal component showing a trigger and content.",
  "messages": [
    {
      "version": "v0.9",
      "createSurface": {
        "surfaceId": "modal-sample-surface",
        "catalogId": "https://a2ui.org/specification/v0_9/catalogs/basic/catalog.json",
        "sendDataModel": true
      }
    },
    {
      "version": "v0.9",
      "updateComponents": {
        "surfaceId": "modal-sample-surface",
        "components": [
          {
            "id": "root",
            "component": "Column",
            "children": ["title", "modal-comp"]
          },
          {
            "id": "title",
            "component": "Text",
            "text": "Modal Component Sample",
            "variant": "h2"
          },
          {
            "id": "modal-comp",
            "component": "Modal",
            "trigger": "open-btn",
            "content": "modal-content"
          },
          {
            "id": "open-btn-text",
            "component": "Text",
            "text": "Open Modal"
          },
          {
            "id": "open-btn",
            "component": "Button",
            "child": "open-btn-text",
            "action": {
              "event": {
                "name": "openModalEvent",
                "context": {}
              }
            }
          },
          {
            "id": "modal-content",
            "component": "Column",
            "children": ["modal-text"]
          },
          {
            "id": "modal-text",
            "component": "Text",
            "text": "This is the content inside the modal."
          }
        ]
      }
    }
  ]
}"#;

const CHAT_MESSAGE: &str = r#"{
  "name": "Chat Message",
  "description": "Example of chat message demonstrating templating and relative paths.",
  "messages": [
    {
      "version": "v0.9",
      "createSurface": {
        "surfaceId": "gallery-chat-message",
        "catalogId": "https://a2ui.org/specification/v0_9/catalogs/basic/catalog.json",
        "sendDataModel": true
      }
    },
    {
      "version": "v0.9",
      "updateComponents": {
        "surfaceId": "gallery-chat-message",
        "components": [
          {
            "id": "root",
            "component": "Card",
            "child": "main-column"
          },
          {
            "id": "main-column",
            "component": "Column",
            "children": ["header", "divider", "messages-list"]
          },
          {
            "id": "header",
            "component": "Row",
            "children": ["channel-icon", "channel-name"],
            "align": "center"
          },
          {
            "id": "channel-icon",
            "component": "Icon",
            "name": "info"
          },
          {
            "id": "channel-name",
            "component": "Text",
            "text": {
              "path": "/channelName"
            },
            "variant": "h3"
          },
          {
            "id": "divider",
            "component": "Divider"
          },
          {
            "id": "messages-list",
            "component": "Column",
            "children": {
              "path": "/messages",
              "componentId": "message-template"
            },
            "align": "start"
          },
          {
            "id": "message-template",
            "component": "Row",
            "children": ["msg-avatar", "msg-content"],
            "align": "start"
          },
          {
            "id": "msg-avatar",
            "component": "Image",
            "url": {
              "path": "avatar"
            },
            "fit": "cover",
            "variant": "avatar"
          },
          {
            "id": "msg-content",
            "component": "Column",
            "children": ["msg-header", "msg-text"]
          },
          {
            "id": "msg-header",
            "component": "Row",
            "children": ["msg-username", "msg-time"],
            "align": "center"
          },
          {
            "id": "msg-username",
            "component": "Text",
            "text": {
              "path": "username"
            },
            "variant": "h4"
          },
          {
            "id": "msg-time",
            "component": "Text",
            "text": {
              "call": "formatDate",
              "args": {
                "value": {
                  "path": "timestamp"
                },
                "format": "h:mm a"
              },
              "returnType": "string"
            },
            "variant": "caption"
          },
          {
            "id": "msg-text",
            "component": "Text",
            "text": {
              "path": "text"
            },
            "variant": "body"
          }
        ]
      }
    },
    {
      "version": "v0.9",
      "updateDataModel": {
        "surfaceId": "gallery-chat-message",
        "value": {
          "channelName": "project-updates",
          "messages": [
            {
              "avatar": "https://images.unsplash.com/photo-1472099645785-5658abf4ff4e?w=40&h=40&fit=crop",
              "username": "Mike Chen",
              "timestamp": "2025-12-15T10:32:00Z",
              "text": "Just pushed the new API changes. Ready for review."
            },
            {
              "avatar": "https://images.unsplash.com/photo-1438761681033-6461ffad8d80?w=40&h=40&fit=crop",
              "username": "Sarah Kim",
              "timestamp": "2025-12-15T10:45:00Z",
              "text": "Great! I'll take a look after standup."
            }
          ]
        }
      }
    }
  ]
}"#;
