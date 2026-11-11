import { test } from "node:test";
import assert from "node:assert/strict";
import {
  dismissIslandCard,
  reconcileIslandCards,
  type IslandCard,
} from "../src/island-queue.ts";
import type { Approval, IslandEvent } from "@agent-hud/protocol";

const request = (id: string, at: string): Approval =>
  ({
    id,
    source: "claude-remote",
    sessionId: `claude-remote:${id}`,
    kind: "question",
    answersQuestion: false,
    createdAt: at,
    questions: [
      { answerKey: "remote", header: id, question: "q", options: [], multiSelect: false },
    ],
  }) as unknown as Approval;

const cards = (queue: IslandCard[]) => queue.map((c) => `${c.type}:${c.id}`);

const news = (id: string, at: string): IslandCard => ({
  type: "news",
  id,
  at,
  event: { id, kind: "completion", at, source: "claude-remote", sessionId: id },
});

test("answering one of several requests keeps the others and the news", () => {
  const previous: IslandCard[] = [
    { type: "request", id: "a", at: "2026-10-28T10:00:00Z" },
    { type: "request", id: "b", at: "2026-10-28T10:01:00Z" },
    news("c", "2026-10-28T10:02:00Z"),
  ];
  // The user answered a; b still waits, c is still news.
  // c was already shown; only fresh events travel in `visible`.
  const next = reconcileIslandCards(
    previous,
    [request("b", "2026-10-28T10:01:00Z")],
    [],
    [],
  );
  assert.deepEqual(cards(next), [
    "request:b",
    "news:c",
  ]);
});

test("dismissing the front card promotes the next waiting one, not a clear-out", () => {
  const previous: IslandCard[] = [
    { type: "request", id: "a", at: "2026-10-28T10:00:00Z" },
    { type: "request", id: "b", at: "2026-10-28T10:01:00Z" },
    news("c", "2026-10-28T10:02:00Z"),
  ];
  const next = dismissIslandCard(previous);
  assert.equal(cards(next).length, 2, "one card goes, the queue stays");
  assert.deepEqual(cards(next), ["request:b", "news:c"]);
});

const completionEvent = (
  id: string,
  at: string,
  sessionId: string,
): IslandEvent => ({
  id,
  kind: "completion",
  at,
  source: "claude-remote",
  sessionId,
});

const completionCard = (
  id: string,
  at: string,
  sessionId: string,
): IslandCard => ({
  type: "news",
  id,
  at,
  event: completionEvent(id, at, sessionId),
});

test("a session's newer completion retires its older one from the queue", () => {
  // A churning loop task finished three rounds; only the latest may stand.
  const previous: IslandCard[] = [
    completionCard("r1", "2026-11-11T10:00:00Z", "claude-remote:aaa"),
    completionCard("r2", "2026-11-11T10:01:00Z", "claude-remote:aaa"),
    completionCard("r3", "2026-11-11T10:02:00Z", "claude-remote:bbb"),
  ];
  const next = reconcileIslandCards(
    previous,
    [],
    [completionEvent("r4", "2026-11-11T10:03:00Z", "claude-remote:aaa")],
    [],
  );
  assert.deepEqual(cards(next), [
    "news:r3",
    "news:r4",
  ]);
});
