//! Claude Code sessions on another machine, mirrored over SSH. The remote
//! poller (host crate) writes turn records beside Claude Code's own files
//! and hook events into the shared inbox, so the collector reads them like
//! any client's callbacks; no other local files belong to this client.
use crate::hooks::{Hooks, Launch, Layout, Observer};
use crate::{Agent, Discovery, PerOs, Source, Support};

/// The remote sessions arrive as hook events; no hook is ever installed.
static HOOKS: Hooks = Hooks {
    settings: None,
    aliases: &[],
    layout: Layout::Groups,
    permission: None,
    completion: None,
    origin: None,
    attention: true,
    compaction: None,
    prefixed_sessions: true,
    launch: PerOs {
        macos: Support::Verified(Launch::Command),
        windows: Support::Verified(Launch::Command),
        linux: Support::Verified(Launch::Command),
    },
};

/// The remote poller (host crate) writes turn records beside Claude Code's
/// own files, in the folder its observer descriptor names; the platform's
/// observers read them like any client plugin's records.
static OBSERVER: Observer = Observer {
    turns: crate::Location::home(
        &["CLAUDE_CONFIG_DIR"],
        &[".claude"],
        "agent-hud/turns",
    ),
    script: crate::Location::home(
        &["CLAUDE_CONFIG_DIR"],
        &[".claude"],
        "agent-hud/turns",
    ),
    marker: "",
    body: "",
    at_key: "observedAtMs",
    states: true,
};

static SOURCES: [Source; 0] = [];

pub static AGENT: Agent = Agent {
    id: "claude-remote",
    name: "Claude (remote)",
    sessions: true,
    hooks: Some(&HOOKS),
    observer: Some(&OBSERVER),
    channel: None,
    program: None,
    replies: None,
    turns: None,
    items: None,
    sources: PerOs {
        macos: Support::Verified(&SOURCES),
        windows: Support::Verified(&SOURCES),
        linux: Support::Verified(&SOURCES),
    },
    discovery: &Discovery::NONE,
};
