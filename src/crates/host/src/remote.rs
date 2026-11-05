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
// A finished turn only raises its card after this much silence. A pause
// shorter than the window is a round-trip inside one task (the model turned,
// the user replied in the remote terminal, the next call started) — raising
// a card for every such pause turned one task's finish into a string of
// notices. Waits (permission asks) skip the window entirely: they are the
// reminder to go answer, not a completion notice.
const DONE_QUIET_WINDOW_MS: i64 = 90_000;

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
    // Finishes holding out their quiet window: the flip was seen, the card
    // waits for the pause to prove real.
    let mut pending_dones: HashMap<String, PendingDone> = HashMap::new();
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
            // The background-jobs table is the roster: every job's state.json
            // (the user's own name for the session, its last words, its fork
            // parent) persists after the process exits, so the HUD list
            // matches claude's own resume list exactly. A bare interactive
            // session the jobs never knew gets no row — it is not part of
            // that list.
            for job in &jobs {
                let Some(sid) = text(job, "sessionId").filter(|s| s.len() >= 8) else {
                    continue;
                };
                let state = text(job, "state").unwrap_or_default();
                let message = text(&job, "detail")
                    .or_else(|| job["output"]["result"].as_str().map(str::to_owned))
                    .unwrap_or_default();
                seen.insert(
                    sid,
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
            }
            // The live table overlays real-time truth on the roster: a job
            // still running in a terminal carries its busy/idle/shell phase,
            // a fresher rename, its real cwd. A job no process holds anymore
            // just keeps its persisted row.
            for row in table {
                let Some(sid) = text(&row, "sessionId").filter(|s| s.len() >= 8) else {
                    continue;
                };
                let Some(existing) = seen.get_mut(&sid) else {
                    continue;
                };
                if let Some(name) = text(&row, "name").filter(|n| !n.is_empty()) {
                    existing.name = name;
                }
                if let Some(status) = text(&row, "status") {
                    existing.status = status;
                }
                let live_stamp =
                    iso_stamp(&row, "statusUpdatedAt").max(iso_stamp(&row, "updatedAt"));
                if live_stamp > 0 {
                    existing.stamp = live_stamp;
                }
                if let Some(started) = row["startedAt"].as_i64() {
                    existing.started = started.clamp(1, now);
                }
                if let Some(cwd) = text(&row, "cwd").filter(|c| !c.is_empty()) {
                    existing.cwd = cwd;
                }
            }
            // A blocked job waits on its user: raise the standing wait card,
            // the same one a permission question used to raise. Read after
            // the overlay, so the wait's stamp is the stamp the retain below
            // will compare against. The retain drops the wait when the
            // session's stamp moves (it resumed) or the row leaves; or_insert
            // keeps re-raising each round free while the wait stands.
            for job in &jobs {
                if text(job, "state").as_deref() != Some("blocked") {
                    continue;
                }
                let Some(sid) = text(job, "sessionId").filter(|s| s.len() >= 8) else {
                    continue;
                };
                if let Some(row) = seen.get(&sid) {
                    waits.entry(sid).or_insert_with(|| Wait {
                        stamp: row.stamp,
                        title: row.name.clone(),
                        message: row.message.clone(),
                    });
                }
            }
            // A session gone from the table loses its record file, which the
            // observers read as a session-forget and the HUD drops the row.
            // A launch also sweeps the records of a previous life: files the
            // roster no longer names — it has no memory of them, so the
            // per-round diff below would never touch them. Only this
            // poller's own records (claude-remote) are swept; a local
            // session's file is another writer's.
            if live.is_empty() {
                if let Ok(entries) = std::fs::read_dir(&root) {
                    for entry in entries.flatten() {
                        let name = entry.file_name();
                        let Some(stem) = name.to_str().and_then(|name| {
                            name.strip_suffix(".json")
                                .filter(|stem| stem.len() == 8)
                        }) else {
                            continue;
                        };
                        let remote_owned = std::fs::read_to_string(entry.path())
                            .ok()
                            .and_then(|bytes| serde_json::from_str::<Value>(&bytes).ok())
                            .and_then(|record| text(&record, "sessionID"))
                            .is_some_and(|id| id.starts_with(&format!("{SOURCE}:")));
                        if remote_owned
                            && !seen.keys().any(|sid| sid.starts_with(stem))
                        {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
            }
            for sid in live.keys() {
                if !seen.contains_key(sid) {
                    let _ = std::fs::remove_file(root.join(format!("{}.json", &sid[..8])));
                }
            }
            // A turn finished: the table itself tells it — busy last pass,
            // done now, no wait standing. But a finish must hold quiet for
            // DONE_QUIET_WINDOW_MS before its card goes up: the table's
            // idle flickers between the turns of a churning run, and each
            // flicker used to raise its own card. Waits skip the window —
            // a question wants an answer now, not in ninety seconds.
            for RaisedDone { sid, stamp, title, cancel_old } in
                advance_dones(&seen, &live, &waits, &mut dones, &mut pending_dones, now)
            {
                if let Some(old) = cancel_old {
                    requests.cancel(&done_id(&sid, old));
                }
                requests.register_native(done_id(&sid, stamp), done_card(&sid, &title, now), now);
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

/// A finished turn waiting out its quiet window before the card goes up.
/// `stamp` is the finish it will announce; if the session moves again
/// (a different stamp), the wait is dropped instead.
struct PendingDone {
    stamp: i64,
    since: i64,
    title: String,
}

/// A finish the quiet window confirmed: the caller registers the card and
/// cancels `cancel_old` (a previous finish's card for the same session).
struct RaisedDone {
    sid: String,
    stamp: i64,
    title: String,
    cancel_old: Option<i64>,
}

/// One round of the done-card quiet window over the seen table. Returns the
/// cards to raise now. Mutates `dones` (the raised-finish memory) and
/// `pending` (the window state).
fn advance_dones(
    seen: &HashMap<String, Row>,
    live: &HashMap<String, Row>,
    waits: &HashMap<String, Wait>,
    dones: &mut HashMap<String, i64>,
    pending: &mut HashMap<String, PendingDone>,
    now: i64,
) -> Vec<RaisedDone> {
    // A session gone from the table is the user's own cleanup — drop any
    // window it had open instead of announcing a finish nobody saw.
    pending.retain(|sid, _| seen.contains_key(sid));
    let mut raised = Vec::new();
    for (sid, row) in seen {
        if row.status == "busy" || waits.contains_key(sid) {
            // Back at work, or asking for input: nothing finished here.
            pending.remove(sid);
            continue;
        }
        match pending.get(sid) {
            Some(p) if p.stamp != row.stamp => {
                // The table moved while quiet — the "finish" is not holding.
                pending.remove(sid);
            }
            Some(p) if now - p.since >= DONE_QUIET_WINDOW_MS => {
                let stamp = p.stamp;
                let title = p.title.clone();
                pending.remove(sid);
                let previous = dones.insert(sid.clone(), stamp);
                raised.push(RaisedDone {
                    sid: sid.clone(),
                    stamp,
                    title,
                    cancel_old: previous.filter(|old| *old != stamp),
                });
            }
            Some(_) => {}
            None => {
                // A fresh flip from busy is the only entry: history idles
                // (a launch, an already-raised finish) never open a window.
                if dones.get(sid) != Some(&row.stamp)
                    && now - row.stamp <= FRESH_COMPLETION
                    && live.get(sid).is_some_and(|old| old.status == "busy")
                {
                    pending.insert(
                        sid.clone(),
                        PendingDone {
                            stamp: row.stamp,
                            since: now,
                            title: row.name.clone(),
                        },
                    );
                }
            }
        }
    }
    raised
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

    fn row(status: &str, stamp: i64) -> Row {
        Row {
            status: status.into(),
            name: "sandbox1-gpu".into(),
            stamp,
            started: 1,
            cwd: "/srv".into(),
            message: String::new(),
            parent: String::new(),
        }
    }

    #[test]
    fn a_finish_waits_out_its_quiet_window_before_the_card_goes_up() {
        let sid = "8c03067d-1234".to_string();
        let mut dones = HashMap::new();
        let mut pending = HashMap::new();
        let mut live = HashMap::new();
        live.insert(sid.clone(), row("busy", 1000));
        let mut seen = HashMap::new();
        seen.insert(sid.clone(), row("idle", 2000));
        let waits = HashMap::new();
        // The flip opens a window; nothing rises yet.
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 100_000).is_empty());
        // A second of quiet left: still nothing.
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 189_999).is_empty());
        // Quiet holds to the full window: one card, for the held stamp.
        let raised = advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 190_000);
        assert_eq!(raised.len(), 1);
        assert_eq!(raised[0].sid, sid);
        assert_eq!(raised[0].stamp, 2000);
        assert_eq!(raised[0].cancel_old, None);
        // Raised already: the same finish never rises twice.
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 190_001).is_empty());
    }

    #[test]
    fn a_finish_that_goes_back_to_busy_never_raises_its_card() {
        let sid = "8c03067d-1234".to_string();
        let mut dones = HashMap::new();
        let mut pending = HashMap::new();
        let mut live = HashMap::new();
        live.insert(sid.clone(), row("busy", 1000));
        let mut seen = HashMap::new();
        seen.insert(sid.clone(), row("idle", 2000));
        let waits = HashMap::new();
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 100_000).is_empty());
        // Back at work mid-window: the window closes.
        *seen.get_mut(&sid).unwrap() = row("busy", 2000);
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 100_001).is_empty());
        assert!(!pending.contains_key(&sid));
        // The same finish still standing idle later: the window re-opens,
        // and its clock restarts — the hold has to be quiet to count.
        *seen.get_mut(&sid).unwrap() = row("idle", 2000);
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 100_002).is_empty());
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 189_002).is_empty());
        let raised = advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 190_002);
        assert_eq!(raised.len(), 1);
        assert_eq!(raised[0].stamp, 2000);
    }

    #[test]
    fn a_finish_that_starts_waiting_never_raises_its_card() {
        let sid = "8c03067d-1234".to_string();
        let mut dones = HashMap::new();
        let mut pending = HashMap::new();
        let mut live = HashMap::new();
        live.insert(sid.clone(), row("busy", 1000));
        let mut seen = HashMap::new();
        seen.insert(sid.clone(), row("idle", 2000));
        let mut waits = HashMap::new();
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 100_000).is_empty());
        // A wait stands up mid-window: the question card goes out at once,
        // and the finish behind it is dropped, not queued behind the answer.
        waits.insert(
            sid.clone(),
            Wait { stamp: 2000, title: "sandbox1-gpu".into(), message: String::new() },
        );
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 100_001).is_empty());
        assert!(!pending.contains_key(&sid));
    }

    #[test]
    fn a_finish_that_moves_stamp_while_quiet_is_dropped() {
        let sid = "8c03067d-1234".to_string();
        let mut dones = HashMap::new();
        let mut pending = HashMap::new();
        let mut live = HashMap::new();
        live.insert(sid.clone(), row("busy", 1000));
        let mut seen = HashMap::new();
        seen.insert(sid.clone(), row("idle", 99_000));
        let waits = HashMap::new();
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 100_000).is_empty());
        // The stamp moves while quiet (the job file's heartbeat writes
        // again): the old hold is not standing, and the round that drops it
        // does not open a new one — that is next round's call.
        *seen.get_mut(&sid).unwrap() = row("idle", 100_000);
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 100_001).is_empty());
        // The heartbeat stops: the newer stamp is still fresh, so it holds
        // from its own round — and the card names it, not the dropped one.
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 100_002).is_empty());
        let raised = advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 190_002);
        assert_eq!(raised.len(), 1);
        assert_eq!(raised[0].stamp, 100_000);
    }

    #[test]
    fn history_idles_never_open_a_window() {
        let sid = "8c03067d-1234".to_string();
        let mut dones = HashMap::new();
        let mut pending = HashMap::new();
        // Last pass was already idle — a launch, or a finish already raised.
        let mut live = HashMap::new();
        live.insert(sid.clone(), row("idle", 2000));
        let mut seen = HashMap::new();
        seen.insert(sid.clone(), row("idle", 2000));
        let waits = HashMap::new();
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 100_000).is_empty());
        assert!(pending.is_empty());
    }

    #[test]
    fn a_stale_finish_never_opens_a_window() {
        let sid = "8c03067d-1234".to_string();
        let mut dones = HashMap::new();
        let mut pending = HashMap::new();
        let mut live = HashMap::new();
        live.insert(sid.clone(), row("busy", 1000));
        let mut seen = HashMap::new();
        // The finish is four minutes old: too old to announce.
        seen.insert(sid.clone(), row("idle", 100_000 - 4 * 60_000));
        let waits = HashMap::new();
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 100_000).is_empty());
        assert!(pending.is_empty());
    }

    #[test]
    fn a_second_finish_cancels_the_first_card() {
        let sid = "8c03067d-1234".to_string();
        let mut dones = HashMap::new();
        let mut pending = HashMap::new();
        let mut live = HashMap::new();
        live.insert(sid.clone(), row("busy", 1000));
        let mut seen = HashMap::new();
        seen.insert(sid.clone(), row("idle", 99_000));
        let waits = HashMap::new();
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 100_000).is_empty());
        let raised = advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 190_000);
        assert_eq!(raised.len(), 1);
        assert_eq!(raised[0].cancel_old, None);
        // Back to work, then a second finish (its stamp is its own moment):
        // its card replaces the first.
        *seen.get_mut(&sid).unwrap() = row("busy", 99_000);
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 190_001).is_empty());
        *seen.get_mut(&sid).unwrap() = row("idle", 190_001);
        assert!(advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 190_002).is_empty());
        let raised = advance_dones(&seen, &live, &waits, &mut dones, &mut pending, 280_002);
        assert_eq!(raised.len(), 1);
        assert_eq!(raised[0].stamp, 190_001);
        assert_eq!(raised[0].cancel_old, Some(99_000));
    }
}
