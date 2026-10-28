import type { Approval, IslandEvent } from "@agent-hud/protocol";

/** Presentation only: Core is the sole producer/deduplicator of event facts. */
export type IslandCard =
  | { type: "request"; id: string; at: string }
  | {
      type: "news";
      id: string;
      at: string;
      event: IslandEvent;
      inUsagePanel?: boolean;
    };

function promoteOldestWaiting(pending: IslandCard[]): IslandCard[] {
  if (pending.length < 2) return pending;
  // Matches IslandAlertQueue.dismiss(): permission requests have waitingSince = request.at
  // and come before non-persistent events (.distantFuture), which keep arrival order.
  const best = pending.reduce((at, card, index) => {
    const waiting = card.type === "request";
    const prior = pending[at],
      priorWaiting = prior.type === "request";
    return waiting &&
      (!priorWaiting || Date.parse(card.at) < Date.parse(prior.at))
      ? index
      : at;
  }, 0);
  return [pending[best], ...pending.filter((_, index) => index !== best)];
}

export function reconcileIslandCards(
  previous: IslandCard[],
  requests: Approval[],
  visible: IslandEvent[],
  newRequests: Approval[] = requests,
  inUsagePanel = false,
): IslandCard[] {
  const frontRemoved =
    previous.length > 0 &&
    previous[0].type === "request" &&
    !requests.some((request) => request.id === previous[0].id);
  const kept = previous.filter(
    (card) =>
      card.type !== "request" ||
      requests.some((request) => request.id === card.id),
  );
  const next = frontRemoved ? promoteOldestWaiting(kept) : kept;
  for (const request of newRequests) {
    if (!next.some((card) => card.id === request.id)) {
      next.push({
        type: "request",
        id: request.id,
        at: request.createdAt,
      });
    }
  }
  for (const event of visible) {
    if (next[0]?.type === "request" && event.kind !== "completion") continue;
    next.push({
      type: "news",
      id: event.id,
      at: event.at,
      event,
      inUsagePanel: event.kind !== "completion" && inUsagePanel,
    });
  }
  // Persistent requests are never evicted by a burst of informational events.
  let news = 0;
  return next.filter((card) => card.type === "request" || ++news <= 32);
}

export function dismissIslandCard(previous: IslandCard[]): IslandCard[] {
  return promoteOldestWaiting(previous.slice(1));
}
