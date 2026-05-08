//! Codex app-server thread status adapter for the Niuma mobile protocol.
//!
//! Codex exposes structured `ThreadStatus` objects. Niuma keeps the mobile
//! wire format as a flat thread-status string, so all status projection must
//! pass through this module.

use serde_json::Value;
use tracing::warn;

/// Convert a Codex thread status payload into a mobile thread-status value.
pub fn normalize_thread_status(status: Option<&Value>, archived: bool) -> String {
    if archived {
        return "archived".to_string();
    }
    let Some(status) = status else {
        return "idle".to_string();
    };
    match status {
        Value::Object(object) => match object.get("type").and_then(Value::as_str) {
            Some("notLoaded") => "notLoaded".to_string(),
            Some("idle") => "idle".to_string(),
            Some("systemError") => "systemError".to_string(),
            Some("active") => active_status(object.get("activeFlags")),
            Some(kind) => {
                warn!("unknown Codex thread status type={kind}");
                "unknown".to_string()
            }
            None => {
                warn!("Codex thread status object missing type");
                "unknown".to_string()
            }
        },
        _ => {
            warn!("unsupported Codex thread status payload={status}");
            "unknown".to_string()
        }
    }
}

fn active_status(flags: Option<&Value>) -> String {
    let flags = flags.and_then(Value::as_array);
    if has_flag(flags, "waitingOnApproval") {
        return "waiting_approval".to_string();
    }
    if has_flag(flags, "waitingOnUserInput") {
        return "pending".to_string();
    }
    "running".to_string()
}

fn has_flag(flags: Option<&Vec<Value>>, expected: &str) -> bool {
    flags
        .into_iter()
        .flatten()
        .any(|flag| flag.as_str() == Some(expected))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::normalize_thread_status;

    #[test]
    fn maps_codex_structured_statuses_to_mobile_statuses() {
        assert_eq!(
            normalize_thread_status(Some(&json!({"type": "idle"})), false),
            "idle"
        );
        assert_eq!(
            normalize_thread_status(Some(&json!({"type": "active", "activeFlags": []})), false),
            "running"
        );
        assert_eq!(
            normalize_thread_status(
                Some(&json!({"type": "active", "activeFlags": ["waitingOnApproval"]})),
                false
            ),
            "waiting_approval"
        );
        assert_eq!(
            normalize_thread_status(
                Some(&json!({"type": "active", "activeFlags": ["waitingOnUserInput"]})),
                false
            ),
            "pending"
        );
        assert_eq!(
            normalize_thread_status(Some(&json!({"type": "idle"})), true),
            "archived"
        );
    }
}
