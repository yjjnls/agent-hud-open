//! The machine's own background jobs' standing wait cards: a local job gone
//! `blocked` waits on its user, and — like a remote session's wait — that
//! deserves a lasting card the pending board holds, not only a 12-second
//! news flash that scrolls away. Reads the same two tables the panel's
//! local board does, on a quiet thread.

use agent_hud_runtime::requests::Requests;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// As quick as a remote poll without hammering claude's own tables.
const POLL: Duration = Duration::from_secs(15);
const SOURCE: &str = "claude";

/// Starts the poller. `requests` holds the island's waiting cards this
/// poller opens for a local input wait, and closes when the session works
/// again.
pub fn spawn(requests: Requests) {
    let started = std::thread::Builder::new()
        .name("agenthud-local-waits".into())
        .spawn(move || run(requests));
    if started.is_err() {
        l10n_warn(
            "本机等待卡线程无法启动",
            "The local wait worker could not start",
        );
    }
}

/// A session's unanswered wait: the words its card shows for as long as the
/// wait lasts. The wait's life is the roster's `blocked` phase itself, not
/// a stamp — the live table's stamp moves on attention events mid-wait.
struct Wait {
    title: String,
    message: String,
}

fn run(requests: Requests) {
    let mut waits: HashMap<String, Wait> = HashMap::new();
    loop {
        let rows = crate::local_jobs::local_jobs();
        // Which waits stand on a living process: a job whose process is gone
        // is a leftover of a stopped run, not someone waiting on the user
        // right now.
        let live: HashSet<String> = claude_dir()
            .and_then(|dir| std::fs::read_dir(dir.join("sessions")).ok())
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| std::fs::read(entry.path()).ok())
                    .filter_map(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                    .filter_map(|row| row["sessionId"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let now = now_ms();
        // Raise a card for each live `blocked` job — the roster's own word
        // for "the turn is over, the user's turn now". The overlay phase
        // would read a frozen busy stamp as still running; the raw state is
        // the signal.
        for row in &rows {
            let Some(sid) = row["sessionId"].as_str().map(str::to_owned) else {
                continue;
            };
            if row["rawState"].as_str() != Some("blocked") || !live.contains(&sid) {
                continue;
            }
            waits.entry(sid).or_insert_with(|| Wait {
                title: row["name"].as_str().unwrap_or_default().to_owned(),
                message: row["message"].as_str().unwrap_or_default().to_owned(),
            });
        }
        // A wait survives the row's flicker: only the session leaving the
        // tables, or the roster moving the job out of `blocked` (back to
        // work, or finished), says the user answered. Re-raising every pass
        // is free while the wait stands — an expired hold comes back, a
        // user-dismissed one stays down.
        waits.retain(|sid, wait| {
            let Some(row) = rows
                .iter()
                .find(|row| row["sessionId"].as_str() == Some(sid.as_str()))
            else {
                return false;
            };
            if row["rawState"].as_str() != Some("blocked") || !live.contains(sid) {
                return false;
            }
            requests.register_native(
                wait_id(sid),
                wait_card(sid, &wait.title, &wait.message, now),
                now,
            );
            true
        });
        let waiting_full: Vec<String> = waits.keys().cloned().collect();
        requests.set_native_waiting(SOURCE, &waiting_full, now);
        std::thread::sleep(POLL);
    }
}

fn claude_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude")))
}

fn wait_id(sid: &str) -> String {
    format!("{SOURCE}-local-wait:{sid}")
}

fn wait_card(sid: &str, title: &str, message: &str, now: i64) -> Value {
    let created = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default();
    let name = if title.is_empty() {
        agent_hud_runtime::l10n::text("Claude", "Claude").into()
    } else {
        title.to_string()
    };
    json!({
        "title": name,
        "source": SOURCE,
        "vendor": agent_hud_runtime::l10n::text("本机 Claude", "Local Claude"),
        "toolName": "claude",
        "input": message,
        "summary": message,
        "sessionId": sid,
        "createdAt": created,
        "allowsAlways": false,
        "kind": "question",
        "answersQuestion": false,
        "answerNote": agent_hud_runtime::l10n::text(
            "本机后台会话：在终端 claude attach 它，或从面板本机会话板点开处理",
            "Local background session: attach it in a terminal, or open it from the panel's local board",
        ),
        "questions": [{
            "answerKey": "local",
            "header": name,
            "question": message,
            "options": [],
            "multiSelect": false,
        }],
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

fn l10n_warn(zh: &str, en: &str) {
    eprintln!("agent-hud local waits: {}", agent_hud_runtime::l10n::text(zh, en));
}
