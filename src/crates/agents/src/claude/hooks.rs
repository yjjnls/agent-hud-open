//! Claude Code's hooks: permission requests with permanent rules, plans and
//! questions; session origin, attention, compaction and completed turns.
use crate::hooks::{
    stop, Completed, Compaction, Completion, Hooks, Launch, Layout, Permission, Reply,
};
use crate::Location;
use crate::{HookEvent, PerOs, Support};
use serde_json::{json, Value};

fn completed(payload: &Value, now_ms: i64) -> Option<Completed> {
    stop(payload, now_ms, false).map(|mut done| {
        done.transcript_path = Some(json!(payload["transcript_path"].as_str()));
        done
    })
}

/// Claude Code's Stop payload omits its final message identity. The platform
/// may supply at most the last MiB of a verified transcript; the schema stays
/// here. A newer user message is a boundary, not permission to reuse an older
/// reply.
fn enrich(event: &mut HookEvent, transcript_tail: &[u8]) -> bool {
    if event.kind != "completion" {
        return false;
    }
    // Claude's sessions carry no source prefix (`prefixed_sessions: false`).
    let session = event.session_id.as_str();
    let tail = &transcript_tail[transcript_tail.len().saturating_sub(1024 * 1024)..];
    for line in tail.split(|byte| *byte == b'\n').rev() {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if record["sessionId"].as_str().is_some_and(|id| id != session) {
            continue;
        }
        match record["type"].as_str() {
            Some("user") => return false,
            Some("assistant") => {}
            _ => continue,
        }
        let message = &record["message"];
        let text = message["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|block| {
                        if block["type"] == "text" {
                            block["text"].as_str()
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        if event.data["message"].is_null() {
            event.data["message"] = json!(text);
        }
        if event.data["model"].is_null() {
            if let Some(model) = message["model"].as_str().filter(|s| !s.trim().is_empty()) {
                event.data["model"] = json!(model);
            }
        }
        return true;
    }
    false
}

pub static HOOKS: Hooks = Hooks {
    settings: Some(Location::home(
        &["CLAUDE_CONFIG_DIR"],
        &[".claude"],
        "settings.json",
    )),
    aliases: &[],
    layout: Layout::Groups,
    permission: Some(Permission {
        timeout: 86_400,
        unanswerable: &[],
        rules: true,
        plans: true,
        reviewer_gate: false,
        settles_in_transcript: true,
        unattended: Some(super::process::unattended),
        reply: Reply::HookSpecificOutput,
    }),
    completion: Some(Completion {
        event: "Stop",
        timeout: 5,
        read: completed,
        enrich: Some(enrich),
        reply: None,
    }),
    origin: Some(super::process::origin),
    attention: true,
    compaction: Some(Compaction {
        // The original compaction hook ran the session-origin entrypoint.
        legacy_flag: Some("--session-origin-hook"),
    }),
    prefixed_sessions: false,
    launch: PerOs {
        macos: Support::Verified(Launch::Command),
        windows: Support::Unverified(Launch::CommandArgs),
        linux: Support::Unverified(Launch::Command),
    },
};
