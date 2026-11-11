import { useEffect, useRef, useState } from "react";
import type { Approval, Snapshot } from "@agent-hud/protocol";
import { usePanelServices } from "./PanelContext";
import { sourceID, vendorKey } from "./model";
import {
  dismissIslandCard,
  reconcileIslandCards,
  type IslandCard,
} from "./island-queue";

/** Presentation only: Core is the sole producer/deduplicator of event facts.
 * Deliberately free of re-exports and alert-view imports: the alert views
 * import these helpers in turn, and a second edge here is a bundling cycle
 * that hoists `reconcileIslandCards` out of reach (ReferenceError at boot). */
export function useIslandNews(
  snapshot: Snapshot,
  held: boolean,
  requests: Approval[],
  shown: string[],
  silenced = false,
  underPointer = true,
  inUsagePanel = false,
) {
  const services = usePanelServices(),
    seen = useRef<Set<string> | null>(null),
    seenRequests = useRef<Set<string> | null>(null),
    picked = useRef<string | null>(null);
  const [queue, setQueue] = useState<IslandCard[]>([]);
  const items = snapshot.islandEvents?.items ?? [];
  const requestIDs = requests.map((request) => request.id).join("\0");
  useEffect(() => {
    const firstRun = seen.current === null;
    const fresh =
      firstRun
        ? []
        : items.filter((event) => !seen.current!.has(event.id));
    // News lands on every screen — the user reads whichever display they are
    // facing, and a card that only the pointer's screen shows reads as
    // "lost" on the rest. Each screen keeps its own seen set (the first run
    // still sets the baseline, so a window waking up never replays the whole
    // backlog); the pointer gates nothing here anymore.
    if (seen.current === null) seen.current = new Set();
    for (const event of items) seen.current.add(event.id);
    if (seen.current.size > 128)
      seen.current = new Set(items.map((event) => event.id));
    const freshRequests =
      seenRequests.current === null
        ? requests
        : requests.filter((request) => !seenRequests.current!.has(request.id));
    if (firstRun || underPointer) {
      if (seenRequests.current === null) seenRequests.current = new Set();
      const activeRequestIds = new Set(requests.map((request) => request.id));
      for (const id of [...seenRequests.current]) {
        if (!activeRequestIds.has(id)) seenRequests.current.delete(id);
      }
      for (const request of requests) seenRequests.current.add(request.id);
    }
    // A finished turn that already holds its lasting card does not need a
    // second, flashier one: the "new reply" flash and the durable card say
    // the same thing about the same conversation, and both showing reads as
    // two news items. The durable card wins — it carries the summary and
    // the acknowledge button.
    const doneSessionKeys = new Set(
      requests
        .filter((request) => request.id.includes("-done:"))
        .map((request) => (request.sessionId ?? "").split(":").pop() ?? ""),
    );
    const visible =
      silenced
        ? []
        : fresh.filter(
            (event) =>
              event.islandAlert !== false &&
              shown.includes(vendorKey(event.source)) &&
              !(event.kind === "completion" &&
                doneSessionKeys.has(
                  (event.sessionId ?? "").split(":").pop() ?? "",
                )) &&
              (event.kind !== "completion" ||
                (services?.sources?.[sourceID(snapshot, event.source)]?.live !==
                  false &&
                  services?.sources?.[sourceID(snapshot, event.source)]
                    ?.reminders !== false)),
          );
    setQueue((previous) =>
      reconcileIslandCards(
        silenced
          ? previous.filter((card) => card.type === "request")
          : previous,
        requests,
        visible,
        // Waiting requests are host-driven state, so every screen holds its
        // own copy and the user can answer from whichever they are on.
        freshRequests,
        inUsagePanel,
      ),
    );
  }, [snapshot.islandEvents?.revision, requestIDs, silenced, underPointer]);
  const current = queue[0];
  // Acknowledging the last card used to collapse the surface out from under
  // the pointer: the queue emptied, the expanded body rendered null, and
  // reading the next card cost another notch hover. A drained queue holds a
  // quiet "all read" row instead, until the pointer leaves or news returns.
  const [drained, setDrained] = useState(false);
  const dismiss = () =>
    setQueue((previous) => {
      const next = dismissIslandCard(previous);
      setDrained(next.length === 0);
      return next;
    });
  useEffect(() => {
    if (queue.length > 0) setDrained(false);
  }, [queue.length]);
  useEffect(() => {
    if (!underPointer) setDrained(false);
  }, [underPointer]);
  useEffect(() => {
    if (!current || held || current.type === "request") return;
    // A completion names a finished turn the user wants to catch before it
    // turns; quota notices repeat, so they may move along quickly.
    // A card the user picked themselves deserves a long look before it
    // turns; unsolicited news turns at its own pace.
    const seconds =
      picked.current === current.id
        ? 30
        : current.event?.kind === "completion"
          ? 12
          : 4;
    const timer = setTimeout(dismiss, seconds * 1000);
    return () => clearTimeout(timer);
  }, [current?.id, held]);
  const select = (id: string) => {
    picked.current = id;
    setQueue((previous) => {
      const at = previous.findIndex((card) => card.id === id);
      if (at <= 0) return previous;
      const next = [...previous];
      [next[0], next[at]] = [next[at], next[0]];
      return next;
    });
  };
  return {
    event: current?.type === "news" ? current.event : undefined,
    inUsagePanel:
      current?.type === "news" ? Boolean(current.inUsagePanel) : false,
    request:
      current?.type === "request"
        ? requests.find((request) => request.id === current.id)
        : undefined,
    queue,
    select,
    dismiss,
    drained,
  };
}
