//! Remote Claude Code sessions over SSH: another machine's live session
//! table (`~/.claude/sessions/*.json`, each live process writes one line)
//! plus its background-job table (`~/.claude/jobs/*/state.json`, every
//! job's own record — the full history the live table loses when a job's
//! process exits), polled on a quiet thread. Turn snapshots land as
//! observer records beside Claude Code's own files — the same channel a
//! client's session-observer plugin writes — so the HUD shows the remote
//! list as claude itself names it: the user's own rename for each
//! session, who waits, who runs, who is done.

use agent_hud_runtime::requests::Requests;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Fresh enough that a session's observation re-report always lands inside
/// lookOf's 10 s toolAt freshness window, so a live remote session holds the
/// working look instead of falling back to the thinking one.
const POLL: Duration = Duration::from_secs(8);
/// The SSH host, as named in the ssh config; the environment wins.
const HOST_ENV: &str = "AGENTHUD_REMOTE_HOST";
const DEFAULT_HOST: &str = "yjj-dev";
/// One session-table read is capped, so a runaway remote cannot stall us.
const SESSIONS_LIMIT: usize = 8 << 20;
const SOURCE: &str = "claude-remote";
/// How recent a table-observed finish must be to raise a card: older than
/// this and the session finished while we were away, so the card would
/// only replay history on every launch.
const FRESH_COMPLETION: i64 = 2 * 60_000;

/// Starts the poller. `requests` holds the island's waiting cards this
/// poller opens for a remote input wait, and closes when the session
/// works again. Observer records wake the collector through their own
/// file watcher.
pub fn spawn(requests: Requests) {
    let host = std::env::var(HOST_ENV).unwrap_or_else(|_| DEFAULT_HOST.into());
    let started = std::thread::Builder::new()
        .name("agenthud-remote".into())
        .spawn(move || run(host, requests));
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
    /// The turn's last words (a job's `detail`), shown beside the session.
    message: String,
    /// The parent session of a branched job (the ⑂ mark in claude's
    /// list); empty when the session is its own root.
    parent: String,
}

/// A session's unanswered wait: the table stamp the wait began at, plus the
/// words its card shows for as long as the wait lasts.
struct Wait {
    stamp: i64,
    title: String,
    message: String,
}

fn run(host: String, requests: Requests) {
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
    // The table as last seen: session id → row.
    let mut live: HashMap<String, Row> = HashMap::new();
    // Sessions waiting on an input or a permission: the remote session id →
    // what the wait looks like. A wait outlives the table's idle/busy
    // flicker — only a new turn (a different stamp) or a gone session says
    // the user answered, however long the wait takes.
    let mut waits: HashMap<String, Wait> = HashMap::new();
    // Sessions whose turn just finished: the remote session id → the stamp
    // frozen at the finish. Each finished turn raises a lasting card, like
    // a wait does — one per session, replaced when a new turn finishes.
    let mut dones: HashMap<String, i64> = HashMap::new();
    // A failed read is quiet — the remote may be asleep — but its first
    // appearance is worth one line, as is its clearing.
    let mut failing = false;
    loop {
        if let Some((table, jobs)) = poll(&host) {
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
                        stamp: iso_stamp(&row, "statusUpdatedAt")
                            .max(iso_stamp(&row, "updatedAt")),
                        started: row["startedAt"].as_i64().unwrap_or(0).clamp(1, now),
                        cwd: text(&row, "cwd").unwrap_or_default(),
                        message: String::new(),
                        parent: String::new(),
                    },
                );
            }
            // The background-jobs table fills in every session the live table
            // no longer names — a finished job's process is gone, but its
            // state.json (the user's own name for it, its last words, its
            // waiting state) persists. A session the table does name wins:
            // its status is live truth.
            for job in jobs {
                let Some(sid) = text(&job, "sessionId").filter(|s| s.len() >= 8) else {
                    continue;
                };
                if seen.contains_key(&sid) {
                    continue;
                }
                let state = text(&job, "state").unwrap_or_default();
                let message = text(&job, "detail")
                    .or_else(|| job["output"]["result"].as_str().map(str::to_owned))
                    .unwrap_or_default();
                seen.insert(
                    sid.clone(),
                    Row {
                        status: if state == "working" { "busy" } else { "idle" }.into(),
                        name: text(&job, "name").unwrap_or_default(),
                        stamp: iso_stamp(&job, "updatedAt"),
                        started: iso_stamp(&job, "createdAt").clamp(1, now),
                        cwd: text(&job, "originCwd").unwrap_or_else(|| cwd_of_scan(&job)),
                        message,
                        parent: text(&job, "forkParentSessionId").unwrap_or_default(),
                    },
                );
                // A blocked job waits on its user: raise the standing wait
                // card, the same one a permission question used to raise.
                // The retain below drops the wait when the job's stamp moves
                // (it resumed) or the job leaves the table; or_insert keeps
                // re-raising each round free while the wait stands.
                if state == "blocked" {
                    if let Some(row) = seen.get(&sid) {
                        waits.entry(sid).or_insert_with(|| Wait {
                            stamp: row.stamp,
                            title: row.name.clone(),
                            message: row.message.clone(),
                        });
                    }
                }
            }
            // A session gone from the table loses its record file, which the
            // observers read as a session-forget and the HUD drops the row.
            for sid in live.keys() {
                if !seen.contains_key(sid) {
                    let _ = std::fs::remove_file(root.join(format!("{}.json", &sid[..8])));
                }
            }
            // A turn finished: the table itself tells it — busy last pass,
            // done now, no wait standing. Only a fresh finish raises a
            // card, so a launch never replays finished history.
            for (sid, row) in &seen {
                if row.status == "busy" || waits.contains_key(sid) {
                    continue;
                }
                if dones.get(sid) == Some(&row.stamp) || now - row.stamp > FRESH_COMPLETION {
                    continue;
                }
                if !live.get(sid).is_some_and(|old| old.status == "busy") {
                    continue;
                }
                let previous = dones.insert(sid.clone(), row.stamp);
                if let Some(old) = previous.filter(|old| *old != row.stamp) {
                    requests.cancel(&done_id(sid, old));
                }
                requests.register_native(
                    done_id(sid, row.stamp),
                    done_card(sid, &row.name, now),
                    now,
                );
            }
            // Turn snapshots: every live session is reported each round, so
            // the HUD's own two-minute quiet rule never ends a working one.
            // The turn id rides the table's status timestamp, which freezes
            // at the moment a session changed phase: stable while a phase
            // lasts, so re-reports never re-open the same turn, and fresh
            // each time a wait ends or a turn completes, so each completion
            // raises its own island card.
            for (sid, row) in &seen {
                let state = match (row.status.as_str(), waits.contains_key(sid)) {
                    ("busy", _) => "running",
                    (_, true) => "waitingForApproval",
                    _ => "completed",
                };
                let turn = format!("{}-{}", &sid[..8], row.stamp);
                report(
                    &root,
                    sid,
                    record(
                        sid,
                        turn,
                        state,
                        &row.name,
                        &row.cwd,
                        row.started,
                        now,
                        row.stamp,
                        &row.message,
                        &row.parent,
                    ),
                );
            }
            live = seen.clone();
            // A wait survives the table's idle/busy flicker: only a moved
            // stamp (the session changed phase — back to work, into a shell,
            // wherever) or a session gone from the table says the wait is
            // over. Its card rides the wait: re-raising it every pass is
            // free while it stands, brings it back when its hold ran out, and
            // stays silent once the user has dismissed it themselves.
            waits.retain(|sid, wait| match seen.get(sid) {
                None => {
                    requests.cancel(&wait_id(sid));
                    false
                }
                Some(row) if row.stamp != wait.stamp => {
                    requests.cancel(&wait_id(sid));
                    false
                }
                Some(_) => {
                    requests.register_native(
                        wait_id(sid),
                        wait_card(sid, &wait.title, &wait.message, now),
                        now,
                    );
                    true
                }
            });
            // A finished-turn card lives the same way: it stands until the
            // session starts a new turn (stamp moved on while running) or
            // leaves the table, and every pass re-raises it — an expired
            // hold comes back, a user-dismissed one stays down.
            dones.retain(|sid, stamp| match seen.get(sid) {
                None => {
                    requests.cancel(&done_id(sid, *stamp));
                    false
                }
                Some(row) if row.status == "busy" && row.stamp != *stamp => {
                    requests.cancel(&done_id(sid, *stamp));
                    false
                }
                Some(row) => {
                    requests.register_native(
                        done_id(sid, *stamp),
                        done_card(sid, &row.name, now),
                        now,
                    );
                    true
                }
            });
            let waiting_full: Vec<String> = waits
                .keys()
                .map(|sid| format!("{SOURCE}:{sid}"))
                .collect();
            requests.set_native_waiting(SOURCE, &waiting_full, now);
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
/// A completed turn names its outcome and its real end (the table's status
/// timestamp, which freezes when the session finished) — what the island's
/// completion tracker needs to raise one fresh card, now, instead of one
/// stale card per long-finished session on the next launch. `message`
/// carries the turn's last words for the panel row; `parent` names the
/// session a branched job forked from (claude's ⑂), empty otherwise.
fn record(
    sid: &str,
    turn: String,
    state: &str,
    title: &str,
    cwd: &str,
    started: i64,
    now: i64,
    ended: i64,
    message: &str,
    parent: &str,
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
        "message": message,
        "parentId": parent,
    });
    if state == "completed" {
        value["outcome"] = json!("completed");
        value["endedAtMs"] = json!(ended.max(1));
    }
    value
}

/// A job timestamp that may be epoch milliseconds (the live table) or an
/// ISO instant (a state.json), unified to milliseconds.
fn iso_stamp(row: &Value, key: &str) -> i64 {
    row[key].as_i64().unwrap_or_else(|| {
        row[key]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp_millis())
            .unwrap_or(0)
    })
}

/// A job's working directory: `originCwd` when present, else roughly
/// decoded from the transcript path's project folder (claude projects
/// name folders after the cwd: `-home-yuanjunjie-code` → `/home/…/code`).
fn cwd_of_scan(job: &Value) -> String {
    let scan = text(job, "linkScanPath").unwrap_or_default();
    let Some((_, dir)) = scan.split_once("/projects/") else {
        return String::new();
    };
    let folder = dir.split('/').next().unwrap_or_default();
    match folder.strip_prefix('-') {
        Some(rest) => rest.replace('-', "/"),
        None => String::new(),
    }
}

/// A waiting card's stable id: one per remote session, so repeated
/// permission or notification events never stack cards.
fn wait_id(sid: &str) -> String {
    format!("{SOURCE}-wait:{sid}")
}

/// The card an unanswered remote wait shows: the session's own name (the
/// question's header tag, since a card without buttons renders no title),
/// the wait's message, and where the answer belongs. There are no answer
/// buttons — the remote terminal is the only place to answer in.
fn wait_card(sid: &str, title: &str, message: &str, now: i64) -> Value {
    let created = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default();
    let name = if title.is_empty() {
        l10n("Claude（远程）", "Claude (remote)")
    } else {
        title.to_string()
    };
    json!({
        "title": name,
        "source": SOURCE,
        "vendor": l10n("远程 Claude", "Remote Claude"),
        "toolName": "claude",
        "input": message,
        "summary": message,
        "sessionId": format!("{SOURCE}:{sid}"),
        "createdAt": created,
        "allowsAlways": false,
        "kind": "question",
        "answersQuestion": false,
        "answerNote": l10n(
            "远程会话：请在远程机器的终端里处理",
            "Remote session: handle it in the remote machine's terminal",
        ),
        "questions": [{
            "answerKey": "remote",
            "header": name,
            "question": message,
            "options": [],
            "multiSelect": false,
        }],
    })
}

/// A finished-turn card's id: one per session per finish (the stamp frozen
/// at the finish), so a repeated Stop never stacks cards but the next
/// finished turn raises a fresh one.
fn done_id(sid: &str, stamp: i64) -> String {
    format!("{SOURCE}-done:{sid}:{stamp}")
}

/// The card a just-finished remote turn shows: the session's own name (the
/// question's header tag), the news, and a note that nothing is being asked
/// here. No answer buttons — it is a notice that stays until the session
/// moves on or the user closes it.
fn done_card(sid: &str, title: &str, now: i64) -> Value {
    let created = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default();
    let name = if title.is_empty() {
        l10n("Claude（远程）", "Claude (remote)")
    } else {
        title.to_string()
    };
    json!({
        "title": name,
        "source": SOURCE,
        "vendor": l10n("远程 Claude", "Remote Claude"),
        "toolName": "claude",
        "input": l10n("本轮运行已结束", "Turn finished"),
        "summary": l10n("本轮运行已结束", "Turn finished"),
        "sessionId": format!("{SOURCE}:{sid}"),
        "createdAt": created,
        "allowsAlways": false,
        "kind": "question",
        "answersQuestion": false,
        "answerNote": l10n(
            "远程会话已完成本轮，无需在此回复",
            "Remote session finished its turn — nothing to answer here",
        ),
        "questions": [{
            "answerKey": "remote",
            "header": name,
            "question": l10n("本轮运行已结束", "Turn finished"),
            "options": [],
            "multiSelect": false,
        }],
    })
}

/// The user interface's language, as the rest of the host picks it.
fn l10n(zh: &str, en: &str) -> String {
    agent_hud_runtime::l10n::text(zh, en).into()
}

/// One combined SSH read: the live session table plus the background-jobs
/// table (every job's `state.json` flattened to one line by a single
/// remote python). None when the remote is unreachable.
fn poll(host: &str) -> Option<(Vec<Value>, Vec<Value>)> {
    let script = format!(
        "head -c {SESSIONS_LIMIT} $HOME/.claude/sessions/*.json 2>/dev/null; \
         echo __JOBS__; \
         python3 - <<'PY'
import glob, json, os
for path in glob.glob(os.path.expanduser('~/.claude/jobs/*/state.json')):
    try:
        with open(path) as f:
            d = json.load(f)
    except Exception:
        continue
    row = {{k: d.get(k) for k in (
        'sessionId', 'name', 'state', 'detail', 'output', 'tokens',
        'createdAt', 'updatedAt', 'forkParentSessionId', 'originCwd',
        'linkScanPath')}}
    print(json.dumps(row, ensure_ascii=False, separators=(',', ':')))
PY"
    );
    let out = Command::new("ssh")
        .args(["-o", "ConnectTimeout=5", "-o", "BatchMode=yes", host, &script])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (table_text, jobs_text) = match text.split_once("__JOBS__\n") {
        Some((before, after)) => (before, after),
        None => (text.as_ref(), ""),
    };
    // Both tables: one JSON object per line, bad lines skipped.
    let parse = |chunk: &str| -> Vec<Value> {
        chunk
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    };
    Some((parse(table_text), parse(jobs_text)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_completed_turn_names_its_outcome_and_a_live_one_does_not() {
        let done = record(
            "8c03067d-1234",
            "8c03067d-1793086819869".into(),
            "completed",
            "sandbox1-gpu",
            "/srv",
            1,
            1000,
            990,
            "提交核验完成",
            "",
        );
        assert_eq!(done["outcome"], "completed");
        assert_eq!(done["endedAtMs"], 990);
        assert_eq!(done["message"], "提交核验完成");
        let live = record(
            "8c03067d-1234",
            "8c03067d-1793086819870".into(),
            "running",
            "sandbox1-gpu",
            "/srv",
            1,
            1000,
            1000,
            "",
            "198ba694-a03e-4b28-84f6-c01f77ae3ae7",
        );
        assert!(live["outcome"].is_null());
        assert!(live["endedAtMs"].is_null());
        assert_eq!(live["parentId"], "198ba694-a03e-4b28-84f6-c01f77ae3ae7");
    }

    #[test]
    fn a_job_timestamp_parses_both_epochs_and_iso_instants() {
        let mut job = serde_json::json!({"updatedAt": "2026-10-26T10:56:04.554Z"});
        assert_eq!(iso_stamp(&job, "updatedAt"), 1793012164554);
        job["updatedAt"] = serde_json::json!(1793012164554_i64);
        assert_eq!(iso_stamp(&job, "updatedAt"), 1793012164554);
        assert_eq!(iso_stamp(&serde_json::json!({}), "updatedAt"), 0);
    }

    #[test]
    fn a_waiting_card_points_at_the_remote_terminal_and_cannot_be_answered_here() {
        let card = wait_card("8c03067d-1234", "sandbox1-gpu", "等待权限审批", 1000);
        assert_eq!(card["sessionId"], "claude-remote:8c03067d-1234");
        assert_eq!(card["kind"], "question");
        assert_eq!(card["answersQuestion"], false);
        // The session's own name rides the question's header tag: a card
        // without answer buttons renders no title of its own.
        let question = &card["questions"][0];
        assert_eq!(question["header"], "sandbox1-gpu");
        assert_eq!(question["question"], "等待权限审批");
        assert!(!card["answerNote"].as_str().unwrap().is_empty());
    }

    #[test]
    fn a_finished_turn_card_names_the_session_and_asks_for_nothing() {
        let card = done_card("8c03067d-1234", "sandbox1-gpu", 1000);
        // A fresh finish gets its own id; repeating it stays the same card.
        assert_eq!(done_id("8c03067d-1234", 990), done_id("8c03067d-1234", 990));
        assert_ne!(done_id("8c03067d-1234", 990), done_id("8c03067d-1234", 991));
        assert_eq!(card["kind"], "question");
        assert_eq!(card["answersQuestion"], false);
        let question = &card["questions"][0];
        assert_eq!(question["header"], "sandbox1-gpu");
        assert!(!question["question"].as_str().unwrap().is_empty());
        assert!(!card["answerNote"].as_str().unwrap().is_empty());
    }
}
