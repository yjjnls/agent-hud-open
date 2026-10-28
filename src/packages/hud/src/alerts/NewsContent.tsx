import type { IslandEvent, Snapshot } from "@agent-hud/protocol";
import { CompletionAlertView } from "./CompletionAlertView";
import { QuotaAlertView, ResetCreditAlertView } from "./QuotaAlertViews";
import { snapshotLanguage } from "@agent-hud/ui/format";

/** The news card's body, by event kind. Lives beside the alert views (not in
 * IslandNews) so the news module stays free of component imports. */
export function NewsContent({
  event,
  snapshot,
  dismiss,
  onStats,
  handoff,
  light = false,
  botLogos = true,
}: {
  event: IslandEvent;
  snapshot: Snapshot;
  dismiss: () => void;
  onStats?: () => void;
  handoff?: (callback?: () => void) => void;
  light?: boolean;
  botLogos?: boolean;
}) {
  const zh = snapshotLanguage(snapshot) === "zh";
  if (event.kind === "completion") {
    return (
      <CompletionAlertView
        event={event}
        snapshot={snapshot}
        dismiss={dismiss}
        onStats={onStats}
        handoff={handoff}
        light={light}
        botLogos={botLogos}
      />
    );
  }
  if (event.kind === "reset-credits") {
    return (
      <ResetCreditAlertView
        event={event}
        snapshot={snapshot}
        dismiss={dismiss}
        onStats={onStats}
        light={light}
        botLogos={botLogos}
        zh={zh}
      />
    );
  }
  return (
    <QuotaAlertView
      event={event}
      dismiss={dismiss}
      onStats={onStats}
      light={light}
      botLogos={botLogos}
      zh={zh}
    />
  );
}
