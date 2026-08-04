use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum ExtensionUiRequestFrame {
    #[serde(rename = "extension_ui_request")]
    Request {
        id: String,
        #[serde(flatten)]
        request: ExtensionUiRequest,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method")]
pub enum ExtensionUiRequest {
    #[serde(rename = "select")]
    Select {
        title: String,
        options: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout: Option<u64>,
    },
    #[serde(rename = "confirm")]
    Confirm {
        title: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout: Option<u64>,
    },
    #[serde(rename = "input")]
    Input {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout: Option<u64>,
    },
    #[serde(rename = "editor", rename_all = "camelCase")]
    Editor {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefill: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt_style: Option<bool>,
    },
    #[serde(rename = "cancel", rename_all = "camelCase")]
    Cancel { target_id: String },
    #[serde(rename = "notify", rename_all = "camelCase")]
    Notify {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        notify_type: Option<NotificationType>,
    },
    #[serde(rename = "setStatus", rename_all = "camelCase")]
    SetStatus {
        status_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status_text: Option<String>,
    },
    #[serde(rename = "setWidget", rename_all = "camelCase")]
    SetWidget {
        widget_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        widget_lines: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        widget_placement: Option<WidgetPlacement>,
    },
    #[serde(rename = "setTitle")]
    SetTitle { title: String },
    #[serde(rename = "set_editor_text")]
    SetEditorText { text: String },
    #[serde(rename = "open_url", rename_all = "camelCase")]
    OpenUrl {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        launch_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instructions: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationType {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WidgetPlacement {
    #[serde(rename = "aboveEditor")]
    AboveEditor,
    #[serde(rename = "belowEditor")]
    BelowEditor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum ExtensionUiResponseFrame {
    #[serde(rename = "extension_ui_response")]
    Response {
        id: String,
        #[serde(flatten)]
        response: ExtensionUiResponse,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtensionUiResponse {
    Value { value: String },
    Confirmed { confirmed: bool },
    Cancelled { timed_out: Option<bool> },
}

#[derive(Serialize)]
#[serde(untagged)]
enum ExtensionUiResponseRef<'a> {
    Value {
        value: &'a str,
    },
    Confirmed {
        confirmed: bool,
    },
    Cancelled {
        cancelled: bool,
        #[serde(rename = "timedOut", skip_serializing_if = "Option::is_none")]
        timed_out: Option<bool>,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ExtensionUiResponseWire {
    Value {
        value: String,
    },
    Confirmed {
        confirmed: bool,
    },
    Cancelled {
        cancelled: bool,
        #[serde(default, rename = "timedOut")]
        timed_out: Option<bool>,
    },
}

impl Serialize for ExtensionUiResponse {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Value { value } => ExtensionUiResponseRef::Value { value }.serialize(serializer),
            Self::Confirmed { confirmed } => ExtensionUiResponseRef::Confirmed {
                confirmed: *confirmed,
            }
            .serialize(serializer),
            Self::Cancelled { timed_out } => ExtensionUiResponseRef::Cancelled {
                cancelled: true,
                timed_out: *timed_out,
            }
            .serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for ExtensionUiResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match ExtensionUiResponseWire::deserialize(deserializer)? {
            ExtensionUiResponseWire::Value { value } => Ok(Self::Value { value }),
            ExtensionUiResponseWire::Confirmed { confirmed } => Ok(Self::Confirmed { confirmed }),
            ExtensionUiResponseWire::Cancelled {
                cancelled: true,
                timed_out,
            } => Ok(Self::Cancelled { timed_out }),
            ExtensionUiResponseWire::Cancelled {
                cancelled: false, ..
            } => Err(de::Error::custom(
                "extension UI cancellation must set cancelled to true",
            )),
        }
    }
}
