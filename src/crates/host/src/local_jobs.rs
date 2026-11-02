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
    // A resumed session gets a fresh job directory, so one session can hold
    // several: keep only each session's freshest job, or the list would show
    // the finished run and the resumed wait as two rows.
    let mut freshest: HashMap<String, (String, Value)> = HashMap::new();
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
        let stamp = job["updatedAt"]
            .as_str()
            .filter(|stamp| !stamp.is_empty())
            .unwrap_or_default();
        let entry_id = entry.file_name().to_string_lossy().into_owned();
        match freshest.get(&session_id) {
            Some((_, kept)) if kept["updatedAt"].as_str() >= Some(stamp) => {
                continue;
            }
            _ => {
                freshest.insert(session_id, (entry_id, job));
            }
        }
    }
    for (session_id, (job_id, job)) in freshest {
        let entry_id = job_id;
        let state = job["state"].as_str().unwrap_or_default();
        let live_row = live.get(&session_id);
        let name = job["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .or_else(|| live_row.and_then(|row| row["name"].as_str()))
            .unwrap_or("claude");
        // claude's own list files jobs under Needs input / Working /
        // Completed, and `claude agents --json` reads the same two tables we
        // do: a live session's busy status is its Working, its idle is the
        // wait, and the roster's done stays a finish even with a live
        // process (a resumed wait reads done until its prompt runs).
        let phase = if state == "done" {
            "done"
        } else if live_row.and_then(|row| row["status"].as_str()) == Some("busy") {
            "running"
        } else {
            "waiting"
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
            "id": entry_id,
            "sessionId": session_id,
            "name": name,
            "state": phase,
            // The roster's own word for the phase (`blocked` = the turn is
            // over and the user's turn now), unfettered by the live table's
            // busy stamp, which freezes when a background turn ends — the
            // wait poller needs the real signal, not the overlay's.
            "rawState": state,
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
