import { test } from "node:test";
import assert from "node:assert/strict";
import {
  dismissIslandCard,
  reconcileIslandCards,
  type IslandCard,
} from "../src/island-queue.ts";
import type { Approval } from "@agent-hud/protocol";

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

test("answering one of several requests keeps the others and the news", () => {
  const previous: IslandCard[] = [
    { type: "request", id: "a", at: "2026-10-28T10:00:00Z" },
    { type: "request", id: "b", at: "2026-10-28T10:01:00Z" },
    { type: "news", id: "c", at: "2026-10-28T10:02:00Z" },
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
    { type: "news", id: "c", at: "2026-10-28T10:02:00Z" },
  ];
  const next = dismissIslandCard(previous);
  assert.equal(cards(next).length, 2, "one card goes, the queue stays");
  assert.deepEqual(cards(next), ["request:b", "news:c"]);
});
