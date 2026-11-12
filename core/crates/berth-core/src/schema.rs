//! JSON Schema for the v1 wire protocol, generated from the Rust types with
//! `schemars`. `berth schema` prints this document and `make schema` writes
//! `docs/schema/berth.schema.json`.

use serde_json::{Value, json};

use crate::{protocol, router, store};

/// The complete protocol schema. Definitions are embedded per top-level type;
/// `schemars` shares nested types through each root's `definitions` map.
pub fn document() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://berth.dev/schema/v1.json",
        "title": "Berth protocol v1",
        "protocol": protocol::PROTOCOL_VERSION,
        "app": protocol::APP_NAME,
        "version": protocol::APP_VERSION,
        "methods": router::METHODS,
        "events": protocol::names::ALL,
        "request": schemars::schema_for!(protocol::Request),
        "response": schemars::schema_for!(protocol::Response),
        "event": schemars::schema_for!(protocol::Event),
        "agent": schemars::schema_for!(crate::model::AgentSurface),
        "notification": schemars::schema_for!(crate::model::Notification),
        "snapshot": schemars::schema_for!(store::Snapshot),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_valid_json_and_lists_every_method() {
        let schema = document();
        assert_eq!(schema["protocol"], protocol::PROTOCOL_VERSION);
        let methods = schema["methods"].as_array().unwrap();
        assert!(methods.iter().any(|m| m == "agent.ack"));
        assert!(methods.iter().any(|m| m == "session.restore"));
        assert!(schema["request"]["properties"]["method"].is_object());
        assert!(schema["snapshot"]["properties"]["workspaces"].is_object());
    }
}
