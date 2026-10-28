//! Remote Claude Code sessions over SSH: another machine's live session
//! table (`~/.claude/sessions/*.json`, each live process writes one line)
//! and its notify log (`~/.config/claude-notify/events.log`), polled on a
//! quiet thread. Turn snapshots land as observer records beside Claude
//! Code's own files — the same channel a client's session-observer plugin
//! writes — because the hook inbox only carries hook facts. A session row
//! carries the user's own name for the session and a `status`
//! (busy/idle/shell), so the HUD shows remote work without a transcript.

use agent_hud_agents::HookEvent;
use agent_hud_runtime::hooks::record_event;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// Fresh enough that a session's observation re-report always lands inside
/// lookOf's 10 s toolAt freshness window, so a live remote session holds the
/// working look instead of falling back to the thinking one.
const POLL: Duration = Duration::from_secs(8);
/// The SSH host, as named in the ssh config; the environment wins.
const HOST_ENV: &str = "AGENTHUD_REMOTE_HOST";
const DEFAULT_HOST: &str = "yjj-dev";
/// Where the remote notify log lives, under the remote home.
const EVENTS_LOG: &str = ".config/claude-notify/events.log";
/// One session-table read is capped, so a runaway remote cannot stall us.
const SESSIONS_LIMIT: usize = 8 << 20;
const SOURCE: &str = "claude-remote";

/// Starts the poller. `wake` runs the collector's data waker, so a fresh
/// record is picked up within the same pass; the observer files also wake
/// it through their own watcher.
pub fn spawn(data_dir: PathBuf, wake: Wake) {
    let host = std::env::var(HOST_ENV).unwrap_or_else(|_| DEFAULT_HOST.into());
    let started = std::thread::Builder::new()
        .name("agenthud-remote".into())
        .spawn(move || run(host, data_dir, wake));
    if started.is_err() {
        l10n_warn("远程会话线程无法启动", "The remote session worker could not start");
    }
}

/// One row of the remote session table, as far as the HUD needs it.
#[derive(Clone)]
struct Row {
    status: String,
    name: String,
    stamp: i64,
    started: i64,
    cwd: String,
}

fn run(host: String, data_dir: PathBuf, wake: Wake) {
    // The observer root the agent's descriptor names, beside Claude Code's
    // own files; each session gets one record file, rewritten in place as
    // its turn moves.
    let root = match std::env::var("CLAUDE_CONFIG_DIR") {
        Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir).join("agent-hud/turns"),
        _ => {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".claude/agent-hud/turns")
        }
    };
    // The log's byte offset: 0 reads only its current size, so the first
    // pass never replays history.
    let mut offset: u64 = 0;
    // The table as last seen: session id → row.
    let mut live: HashMap<String, Row> = HashMap::new();
    // Sessions waiting on an input or a permission: their record reads
    // waiting until the user answers, however long that takes.
    let mut waiting: HashSet<String> = HashSet::new();
    // A failed read is quiet — the remote may be asleep — but its first
    // appearance is worth one line, as is its clearing.
    let mut failing = false;
    loop {
        if let Some((table, log, size)) = poll(&host, offset) {
            if failing {
                l10n_warn(
                    &format!("远程 {host} 恢复读取"),
                    &format!("The remote {host} reads again"),
                );
                failing = false;
            }
            let now = now_ms();
            let mut seen: HashMap<String, Row> = HashMap::new();
            for row in table {
                let Some(sid) = text(&row, "sessionId").filter(|s| s.len() >= 8) else {
                    continue;
                };
                seen.insert(
                    sid,
                    Row {
                        status: text(&row, "status").unwrap_or_else(|| "idle".into()),
                        name: text(&row, "name").unwrap_or_default(),
                        stamp: row["statusUpdatedAt"]
                            .as_i64()
                            .or_else(|| row["updatedAt"].as_i64())
                            .unwrap_or(0),
                        started: row["startedAt"].as_i64().unwrap_or(0).clamp(1, now),
                        cwd: text(&row, "cwd").unwrap_or_default(),
                    },
                );
            }
            // A session gone from the table loses its record file, which the
            // observers read as a session-forget and the HUD drops the row.
            for sid in live.keys() {
                if !seen.contains_key(sid) {
                    let _ = std::fs::remove_file(root.join(format!("{}.json", &sid[..8])));
                }
            }
            // Turn snapshots: every live session is reported each round, so
            // the HUD's own two-minute quiet rule never ends a working one.
            for (sid, row) in &seen {
                let state = match (row.status.as_str(), waiting.contains(sid)) {
                    ("busy", _) => "running",
                    (_, true) => "waitingForApproval",
                    _ => "completed",
                };
                let turn = if state == "running" {
                    format!("{}-{}", &sid[..8], row.stamp)
                } else {
                    format!("{}-wait", &sid[..8])
                };
                report(
                    &root,
                    sid,
                    record(sid, turn, state, None, &row.name, &row.cwd, row.started, now),
                );
            }
            live = seen.clone();
            for line in log {
                let Some(sid) = text(&line, "sid").filter(|s| s.len() >= 8) else {
                    continue;
                };
                let Some((plain, row)) = seen
                    .iter()
                    .find(|(uuid, _)| uuid.starts_with(&sid))
                    .map(|(k, v)| (k.clone(), v))
                else {
                    continue; // An event for a session the table no longer names.
                };
                match text(&line, "event").as_deref() {
                    Some("PermissionRequest") | Some("Notification") => {
                        // The balloon over the Clawd rides the hook inbox;
                        // the waiting state rides the observer record.
                        waiting.insert(plain.clone());
                        let message = text(&line, "msg").unwrap_or_else(|| {
                            if text(&line, "event").as_deref() == Some("PermissionRequest") {
                                "等待权限审批".into()
                            } else {
                                "等待输入".into()
                            }
                        });
                        if record_event(
                            &data_dir,
                            &attention_event(&plain, message, now),
                        )
                        .is_ok()
                        {
                            wake();
                        }
                        if row.status != "busy" {
                            report(
                                &root,
                                &plain,
                                record(
                                    &plain,
                                    format!("{}-wait", &sid[..8]),
                                    "waitingForApproval",
                                    None,
                                    &row.name,
                                    &row.cwd,
                                    row.started,
                                    now,
                                ),
                            );
                        }
                    }
                    Some("Stop") if row.status != "busy" => {
                        // The turn is done; the next table read confirms it,
                        // this makes it immediate.
                        waiting.remove(&plain);
                        report(
                            &root,
                            &plain,
                            record(
                                &plain,
                                format!("{}-wait", &sid[..8]),
                                "completed",
                                None,
                                &row.name,
                                &row.cwd,
                                row.started,
                                now,
                            ),
                        );
                    }
                    _ => {}
                }
            }
            // A wait survives only while the session idles unanswered; busy
            // turns keep their own heartbeat, gone sessions end.
            waiting.retain(|sid| seen.get(sid).is_some_and(|row| row.status == "idle"));
            offset = size;
        } else if !failing {
            failing = true;
            l10n_warn(
                &format!("远程 {host} 暂时读不到（每 8 秒重试）"),
                &format!("The remote {host} is unreachable (retrying every 8 s)"),
            );
        }
        std::thread::sleep(POLL);
    }
}

/// Writes one observer record, atomically, as `{sid-prefix}.json`. The
/// record's bytes changing is what renews the turn at the HUD, so the
/// observed timestamp moves every round while a session stays reported.
fn report(root: &Path, sid: &str, record: Value) -> bool {
    if std::fs::create_dir_all(root).is_err() {
        return false;
    }
    let temporary = root.join(format!(".{}.tmp", std::process::id()));
    let path = root.join(format!("{}.json", &sid[..8]));
    let ok = std::fs::write(&temporary, record.to_string().as_bytes()).is_ok()
        && std::fs::rename(&temporary, &path).is_ok();
    if !ok {
        let _ = std::fs::remove_file(&temporary);
    }
    ok
}

/// The observer record the platform's parser expects: version 1, one turn.
fn record(
    sid: &str,
    turn: String,
    state: &str,
    stop: Option<&str>,
    title: &str,
    cwd: &str,
    started: i64,
    now: i64,
) -> Value {
    let mut value = json!({
        "version": 1,
        "sessionID": format!("{SOURCE}:{sid}"),
        "turnID": turn,
        "startedAtMs": started.max(1),
        "observedAtMs": now,
        "state": state,
        "title": title,
        "workspace": cwd,
    });
    if let Some(stop) = stop {
        value["stopReason"] = json!(stop);
    }
    value
}

/// A permission question or an input wait: the balloon over the Clawd.
fn attention_event(sid: &str, message: String, now: i64) -> HookEvent {
    HookEvent {
        source: SOURCE.into(),
        session_id: format!("{SOURCE}:{sid}"),
        at_ms: now,
        kind: "attention".into(),
        data: json!({"message": message}),
    }
}

/// One combined SSH read: the session table, then the log's size and any
/// bytes past our offset. None when the remote is unreachable.
fn poll(host: &str, offset: u64) -> Option<(Vec<Value>, Vec<Value>, u64)> {
    let tail = if offset > 0 {
        format!("tail -c +{} \"$HOME/{EVENTS_LOG}\" 2>/dev/null", offset + 1)
    } else {
        String::new()
    };
    let script = format!(
        "head -c {SESSIONS_LIMIT} $HOME/.claude/sessions/*.json 2>/dev/null; \
         echo __EVENTS__; \
         wc -c < \"$HOME/{EVENTS_LOG}\" 2>/dev/null; \
         {tail}"
    );
    let out = Command::new("ssh")
        .args(["-o", "ConnectTimeout=5", "-o", "BatchMode=yes", host, &script])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (table_text, rest) = match text.split_once("__EVENTS__\n") {
        Some((before, after)) => (before, after),
        None => (text.as_ref(), ""),
    };
    // The session table: one JSON object per line, bad lines skipped.
    let table = table_text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    // The log: its size first, then whole lines only (a torn tail waits).
    let mut lines = rest.lines();
    let size = lines
        .next()
        .and_then(|l| l.trim().parse::<u64>().ok())
        .unwrap_or(offset);
    let mut log_lines: Vec<&str> = lines.collect();
    if !rest.ends_with('\n') {
        log_lines.pop(); // The last line is torn; it travels next round.
    }
    let log: Vec<Value> = log_lines
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    Some((table, log, size))
}

fn text(row: &Value, key: &str) -> Option<String> {
    row[key].as_str().map(str::to_owned)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

fn l10n_warn(zh: &str, en: &str) {
    eprintln!("agent-hud remote: {}", agent_hud_runtime::l10n::text(zh, en));
}
