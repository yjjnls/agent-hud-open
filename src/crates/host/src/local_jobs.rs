//! The machine's own background claude jobs, for the panel's local section:
//! `~/.claude/jobs/*/state.json` is the roster (it survives the process), and
//! `~/.claude/sessions/*.json` overlays real-time truth — the same two tables
//! the remote poller merges over SSH, read directly from disk.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

fn claude_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude")))
}

#[tauri::command]
/// The jobs as claude's own resume list sees them: a finished job keeps its
/// persisted row, a live one carries its phase, an empty one waits for its
/// prompt. Newest activity first.
pub fn local_jobs() -> Vec<Value> {
    let Some(dir) = claude_dir() else {
        return Vec::new();
    };
    let mut live: HashMap<String, Value> = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir.join("sessions")) {
        for entry in entries.flatten() {
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            let Ok(row) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            if let Some(sid) = row["sessionId"].as_str().map(str::to_owned) {
                live.insert(sid, row);
            }
        }
    }
    let mut rows = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir.join("jobs")) else {
        return rows;
    };
    for entry in entries.flatten() {
        let Ok(bytes) = std::fs::read(entry.path().join("state.json")) else {
            continue;
        };
        let Ok(job) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let Some(session_id) = job["sessionId"].as_str().map(str::to_owned) else {
            continue;
        };
        let state = job["state"].as_str().unwrap_or_default();
        let tempo = job["tempo"].as_str().unwrap_or_default();
        let live_row = live.get(&session_id);
        let name = job["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .or_else(|| live_row.and_then(|row| row["name"].as_str()))
            .unwrap_or("claude");
        // claude's own list files jobs under Needs input / Working /
        // Completed; the overlay's busy phase confirms a working one. A
        // "working" job whose tempo has gone idle sits waiting for its
        // prompt, not running.
        let phase = match state {
            "done" => "done",
            "blocked" => "waiting",
            "working" if tempo != "idle" => "running",
            _ => {
                if live_row.and_then(|row| row["status"].as_str()) == Some("busy") {
                    "running"
                } else {
                    "waiting"
                }
            }
        };
        let text = job["detail"]
            .as_str()
            .filter(|detail| !detail.trim().is_empty())
            .or_else(|| job["output"]["result"].as_str())
            .unwrap_or_default();
        let stamp = live_row
            .and_then(|row| row["statusUpdatedAt"].as_str())
            .filter(|stamp| !stamp.is_empty())
            .or_else(|| live_row.and_then(|row| row["updatedAt"].as_str()))
            .or_else(|| job["updatedAt"].as_str())
            .unwrap_or_default();
        rows.push(json!({
            "id": entry.file_name().to_string_lossy(),
            "sessionId": session_id,
            "name": name,
            "state": phase,
            "message": text,
            "stamp": stamp,
            "startedAt": job["createdAt"].as_str().unwrap_or_default(),
            "cwd": live_row
                .and_then(|row| row["cwd"].as_str())
                .filter(|cwd| !cwd.is_empty())
                .or_else(|| job["originCwd"].as_str())
                .unwrap_or_default(),
        }));
    }
    rows.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
    rows
}

/// Waits first, then the working, then the finished — each group newest
/// activity first, like claude's own resume list.
fn sort_key(row: &Value) -> (i32, std::cmp::Reverse<&str>) {
    (
        match row["state"].as_str() {
            Some("waiting") => 0,
            Some("running") => 1,
            _ => 2,
        },
        std::cmp::Reverse(row["stamp"].as_str().unwrap_or_default()),
    )
}
