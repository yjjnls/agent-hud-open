import { test } from "node:test";
import assert from "node:assert/strict";
import {
  groupRemoteSessions,
  remoteGroupKey,
  REMOTE_OTHER_GROUP,
} from "../src/remote-groups.ts";
import type { Session } from "@agent-hud/protocol";

const session = (title: string, at = 0, state = "idle"): Session =>
  ({
    id: title,
    source: "claude-remote",
    title,
    state,
    updatedAt: new Date(at || Date.UTC(2026, 0, 1)).toISOString(),
  }) as unknown as Session;

const rank = (session: Session) =>
  session.state === "running" ? 1 : session.state === "waiting" ? 0 : 2;

test("a session's own name carries its group", () => {
  // The user's own category words stay whole.
  assert.equal(remoteGroupKey("调研-agentenv"), "调研");
  assert.equal(remoteGroupKey("问题排查-deepinfer1-15s"), "问题排查");
  assert.equal(remoteGroupKey("梳理-llm-gateway"), "梳理");
  // A numbered host folds into its base; a distinct word stays its own.
  assert.equal(remoteGroupKey("deepinfer2-1026需求"), "deepinfer");
  assert.equal(remoteGroupKey("deepinfer bot"), "deepinfer");
  assert.equal(remoteGroupKey("sandbox3-user-quota"), "sandbox");
  assert.equal(remoteGroupKey("sandbox告警处理bot"), "sandbox");
  assert.equal(remoteGroupKey("opensandbox"), "opensandbox");
  assert.equal(remoteGroupKey("tmux-persistence session count"), "tmux");
  assert.equal(remoteGroupKey("Claude 是否支持 mod"), "claude");
  assert.equal(remoteGroupKey("claude会话"), "claude");
});

test("groups fold singletons into one tail group", () => {
  const groups = groupRemoteSessions(
    [
      session("调研-agentenv", 1),
      session("调研-sysbox", 2),
      session("llm-audit", 3),
      session("opensandbox", 4),
    ],
    rank,
  );
  assert.deepEqual(
    groups.map((group) => group.key),
    ["调研", REMOTE_OTHER_GROUP],
  );
  assert.deepEqual(
    groups[1].sessions.map((session) => session.title),
    ["opensandbox", "llm-audit"],
  );
});

test("live groups lead, and each group keeps the board's own order", () => {
  const groups = groupRemoteSessions(
    [
      session("sandbox1-gpu", 10),
      session("问题排查-deepinfer1-15s", 20, "running"),
      session("问题排查-sandbox1", 25),
      session("sandbox4-egress", 30, "waiting"),
      session("deepinfer2-ui", 40),
      session("deepinfer bot", 50),
    ],
    rank,
  );
  assert.deepEqual(
    groups.map((group) => group.key),
    ["sandbox", "问题排查", "deepinfer"],
  );
  // Inside a group waits first, then the fresher row.
  assert.deepEqual(
    groups[0].sessions.map((session) => session.title),
    ["sandbox4-egress", "sandbox1-gpu"],
  );
  assert.deepEqual(
    groups[2].sessions.map((session) => session.title),
    ["deepinfer bot", "deepinfer2-ui"],
  );
});

test("a finished group yields to a running one, freshness breaks ties", () => {
  const groups = groupRemoteSessions(
    [
      session("tmux-persistence session count", 100),
      session("tmux display resize glitch", 110),
      session("调研-agentenv", 200, "running"),
      session("调研-sysbox", 210),
      session("glm-package-balance-api", 300),
      session("glm-package-balance-api ⑂", 310),
    ],
    rank,
  );
  assert.deepEqual(
    groups.map((group) => group.key),
    ["调研", "glm", "tmux"],
  );
});
