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
    const fresh =
      seen.current === null
        ? []
        : items.filter((event) => !seen.current!.has(event.id));
    if (seen.current === null) seen.current = new Set();
    for (const event of items) seen.current.add(event.id);
    if (seen.current.size > 128)
      seen.current = new Set(items.map((event) => event.id));
    const freshRequests =
      seenRequests.current === null
        ? requests
        : requests.filter((request) => !seenRequests.current!.has(request.id));
    if (seenRequests.current === null) seenRequests.current = new Set();
    const activeRequestIds = new Set(requests.map((request) => request.id));
    for (const id of [...seenRequests.current]) {
      if (!activeRequestIds.has(id)) seenRequests.current.delete(id);
    }
    for (const request of requests) seenRequests.current.add(request.id);
    const visible =
      silenced || !underPointer
        ? []
        : fresh.filter(
            (event) =>
              event.islandAlert !== false &&
              shown.includes(vendorKey(event.source)) &&
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
        underPointer ? freshRequests : [],
        inUsagePanel,
      ),
    );
  }, [snapshot.islandEvents?.revision, requestIDs, silenced, underPointer]);
  const current = queue[0];
  const dismiss = () => setQueue(dismissIslandCard);
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
  };
}
