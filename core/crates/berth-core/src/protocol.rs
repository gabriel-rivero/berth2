//! Wire types. The protocol is the contract; these structs are its source of
//! truth and JSON Schema is generated from them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::CoreError;

/// `{"id":1,"method":"surface.split","params":{…}}`
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Request {
    #[serde(default)]
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// `{"id":1,"ok":true,"result":{…}}` / `{"id":1,"ok":false,"error":{…}}`
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Response {
    pub id: Value,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    MethodNotFound,
    NotFound,
    InvalidState,
    PermissionDenied,
    PayloadTooLarge,
    Unsupported,
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::MethodNotFound => "method_not_found",
            Self::NotFound => "not_found",
            Self::InvalidState => "invalid_state",
            Self::PermissionDenied => "permission_denied",
            Self::PayloadTooLarge => "payload_too_large",
            Self::Unsupported => "unsupported",
            Self::Internal => "internal",
        }
    }
}

/// `{"event":"agent.state_changed","seq":42,"ts":"…","data":{…}}`
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    pub event: String,
    /// Monotonic per process. Clients detect gaps, not reordering.
    #[serde(default)]
    pub seq: u64,
    /// RFC3339 UTC with milliseconds.
    #[serde(default)]
    pub ts: String,
    #[serde(default)]
    pub data: Value,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }
    pub fn err(id: Value, err: CoreError) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(ErrorBody {
                code: err.code,
                message: err.message,
                data: Value::Null,
            }),
        }
    }
}

impl From<CoreError> for Response {
    fn from(err: CoreError) -> Self {
        Self::err(Value::Null, err)
    }
}

/// Event names, collected so the shell, CLI, and schema generator agree.
pub mod names {
    pub const HELLO: &str = "hello";
    pub const WORKSPACE_CREATED: &str = "workspace.created";
    pub const WORKSPACE_CLOSED: &str = "workspace.closed";
    pub const WORKSPACE_CHANGED: &str = "workspace.changed";
    pub const WORKSPACE_SELECTED: &str = "workspace.selected";
    pub const WORKSPACE_MOVED: &str = "workspace.moved";
    pub const GROUP_CREATED: &str = "group.created";
    pub const GROUP_UPDATED: &str = "group.updated";
    pub const GROUP_DELETED: &str = "group.deleted";
    pub const PANE_FOCUS_CHANGED: &str = "pane.focus_changed";
    pub const SURFACE_CREATED: &str = "surface.created";
    pub const SURFACE_CLOSED: &str = "surface.closed";
    pub const SURFACE_TITLE_CHANGED: &str = "surface.title_changed";
    pub const SURFACE_PWD_CHANGED: &str = "surface.pwd_changed";
    pub const SURFACE_BELL: &str = "surface.bell";
    pub const SURFACE_PROGRESS: &str = "surface.progress";
    pub const SURFACE_EXITED: &str = "surface.exited";
    pub const AGENT_CREATED: &str = "agent.created";
    pub const AGENT_DELETED: &str = "agent.deleted";
    pub const AGENT_STATE_CHANGED: &str = "agent.state_changed";
    pub const AGENT_ATTENTION_CHANGED: &str = "agent.attention_changed";
    pub const AGENT_PROGRESS: &str = "agent.progress";
    pub const NOTIFICATION_CREATED: &str = "notification.created";
    pub const NOTIFICATION_ACKED: &str = "notification.acked";

    pub const ALL: &[&str] = &[
        HELLO,
        WORKSPACE_CREATED,
        WORKSPACE_CLOSED,
        WORKSPACE_CHANGED,
        WORKSPACE_SELECTED,
        WORKSPACE_MOVED,
        GROUP_CREATED,
        GROUP_UPDATED,
        GROUP_DELETED,
        PANE_FOCUS_CHANGED,
        SURFACE_CREATED,
        SURFACE_CLOSED,
        SURFACE_TITLE_CHANGED,
        SURFACE_PWD_CHANGED,
        SURFACE_BELL,
        SURFACE_PROGRESS,
        SURFACE_EXITED,
        AGENT_CREATED,
        AGENT_DELETED,
        AGENT_STATE_CHANGED,
        AGENT_ATTENTION_CHANGED,
        AGENT_PROGRESS,
        NOTIFICATION_CREATED,
        NOTIFICATION_ACKED,
    ];
}

pub const PROTOCOL_VERSION: u32 = 1;
pub const APP_NAME: &str = "Berth";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_shapes_are_stable() {
        let ok = Response::ok(
            serde_json::json!(1),
            serde_json::json!({"surface_id":"srf_ab"}),
        );
        let s = serde_json::to_string(&ok).unwrap();
        assert_eq!(s, r#"{"id":1,"ok":true,"result":{"surface_id":"srf_ab"}}"#);

        let err = Response::err(
            serde_json::json!("x"),
            CoreError::new(ErrorCode::NotFound, "no surface"),
        );
        let s = serde_json::to_string(&err).unwrap();
        assert_eq!(
            s,
            r#"{"id":"x","ok":false,"error":{"code":"not_found","message":"no surface"}}"#
        );
    }

    #[test]
    fn error_codes_are_snake_case() {
        let s = serde_json::to_string(&ErrorCode::MethodNotFound).unwrap();
        assert_eq!(s, "\"method_not_found\"");
    }

    #[test]
    fn request_defaults_id_and_params() {
        let r: Request = serde_json::from_str(r#"{"method":"system.ping"}"#).unwrap();
        assert!(r.id.is_null());
        assert!(r.params.is_null());
    }
}
