// Prints the JSON a shell sees for `workspace.layout`. Handy when changing the
// protocol: the AppKit side decodes exactly this shape.
use berth_core::{AppState, NullOps, Request, dispatch};
use serde_json::{Value, json};

fn call(state: &mut AppState, method: &str, params: Value) -> Value {
    let request = Request {
        id: json!(1),
        method: method.into(),
        params,
    };
    dispatch(state, &NullOps, &request)
        .result
        .unwrap_or(Value::Null)
}

fn main() {
    let mut state = AppState::default();
    let workspace = call(&mut state, "workspace.create", json!({"name": "main"}));
    let workspace_id = workspace["workspace"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let surface_id = call(&mut state, "surface.list", json!({}))["surfaces"][0]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    call(
        &mut state,
        "surface.split",
        json!({"surface_id": surface_id, "direction": "down"}),
    );
    let layout = call(
        &mut state,
        "workspace.layout",
        json!({"workspace_id": workspace_id}),
    );
    println!("{}", serde_json::to_string_pretty(&layout).unwrap());
}
