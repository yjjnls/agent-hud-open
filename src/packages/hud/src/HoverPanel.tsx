import {
  memo,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import {
  ChartNoAxesColumnIncreasing,
  CircleArrowDown,
  Settings,
} from "lucide-react";
import { motion, MotionConfig } from "motion/react";
import type {
  Quota,
  RequestDecision,
  Session,
  Snapshot,
} from "@agent-hud/protocol";
import { Button } from "@agent-hud/ui/components/button";
import {
  defaultHUDConfig,
  inRowOrder,
  quotaAccountKey,
  quotaRowHidden,
  quotaRowKey,
  vendorKey,
  type HUDConfig,
  type HUDUpdateNotice,
} from "./model";
import { usePanelServices } from "./PanelContext";
import { useGround } from "./ground";
import { FittingName } from "./panel/FittingName";
import { SourceBalances, SourceProviderBlock } from "./panel/QuotaSection";
import { SourceSessions } from "./panel/SessionsSection";
import {
  densityMetrics,
  inPanelOrder,
  palette,
  type Logo,
} from "./panel/theme";
import { SourceTokenChart } from "./panel/TokenChartSection";
import { SourceIndexing } from "./panel/IndexingSection";
import { clientDisplayName } from "./model";
import { ApprovalContent } from "./permissions/ApprovalContent";
import { age, snapshotLanguage } from "@agent-hud/ui/format";
import { groupRemoteSessions, REMOTE_OTHER_GROUP } from "./remote-groups";

export { SOURCE_PANEL } from "./panel/theme";

export interface SourceHoverPanelProps {
  config?: HUDConfig;
  snapshot: Snapshot;
  focusVendor?: string;
  light: boolean;
  Logo: Logo;
  onConfigure?: () => void;
  onStats?: () => void;
  /** The product's newer version, as Open's HUD showed it: an arrow beside
   * Statistics that opens the update in Settings. */
  update?: HUDUpdateNotice | null;
  onUpdate?: () => void;
  /** Answers a pending request from the panel: the same channel the island
   * card uses, so a wait or a finished-turn notice can be dismissed here. */
  onDecide?: (id: string, decision: RequestDecision) => void | Promise<void>;
  maxHeight?: number;
  active?: boolean;
  inlineAlert?: ReactNode;
}

/** Default HoverPanelView arrangement. Missing account plans and per-cycle measures remain unknown. */
export const SourceHoverPanel = memo(function SourceHoverPanel({
  config = defaultHUDConfig,
  snapshot,
  focusVendor,
  light,
  Logo,
  onConfigure,
  onStats,
  update,
  onUpdate,
  onDecide,
  maxHeight,
  active = true,
  inlineAlert,
}: SourceHoverPanelProps) {
  const colors = palette(useGround(light));
  const services = usePanelServices();
  const [draftError, setDraftError] = useState<string | null>(null);
  const panelElement = useRef<HTMLDivElement>(null),
    [contentWidth, setContentWidth] = useState<number>();
  useEffect(() => {
    const element = panelElement.current;
    if (!element) return;
    const measure = () => setContentWidth(element.clientWidth);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const rows = useMemo(
    () =>
      snapshot.quotas.filter(
        (row) =>
          !focusVendor || vendorKey(row.vendor) === vendorKey(focusVendor),
      ),
    [snapshot.quotas, focusVendor],
  );
  const rowsByVendor = useMemo(() => {
    const groups = new Map<string, Quota[]>();
    for (const row of rows) {
      const group = groups.get(row.vendor) ?? [];
      group.push(row);
      groups.set(row.vendor, group);
    }
    return groups;
  }, [rows]);
  const vendorNames = useMemo(
    () =>
      [
        ...new Set([
          ...rows.map((row) => row.vendor),
          ...snapshot.providers
            .filter(
              (provider) =>
                provider.detected &&
                provider.enabled &&
                (!focusVendor ||
                  vendorKey(provider.id) === vendorKey(focusVendor)) &&
                !snapshot.billing.some(
                  (account) =>
                    vendorKey(account.vendor) === vendorKey(provider.id),
                ),
            )
            .map((provider) => provider.id),
        ]),
      ].filter((vendor) => !config.panelLayout.blocks?.[vendor]?.hidden),
    [
      rows,
      snapshot.providers,
      snapshot.billing,
      focusVendor,
      config.panelLayout,
    ],
  );
  const billing = snapshot.billing.filter(
    (account) =>
      !focusVendor || vendorKey(account.vendor) === vendorKey(focusVendor),
  );
  const density = densityMetrics[config.layoutDensity] ?? densityMetrics.normal;
  const blocks = inPanelOrder<{ key: string; content: ReactNode }>(
    [
      ...vendorNames.map((vendor) => ({
        key: vendor,
        content: (
          <SourceProviderBlock
            vendor={vendor}
            rows={inRowOrder(
              (rowsByVendor.get(vendor) ?? []).filter((row) => {
                const block = config.panelLayout.blocks?.[vendor];
                return (
                  !quotaRowHidden(row, block?.hiddenRows ?? []) &&
                  !(block?.hiddenAccounts ?? []).includes(
                    quotaAccountKey(row),
                  ) &&
                  (!block?.accounts?.length ||
                    block.accounts.includes(quotaAccountKey(row)))
                );
              }),
              config.panelLayout.blocks?.[vendor]?.rows ?? [],
            ).map((row) => ({ ...row, key: quotaRowKey(row) }))}
            layout={config.panelLayout.blocks?.[vendor]}
            snapshot={snapshot}
            active={active}
            now={Date.parse(snapshot.generatedAt)}
            light={light}
            Logo={Logo}
            showReset={config.resetCountdown}
            accountInfo={snapshot.accounts}
            onStats={onStats}
          />
        ),
      })),
      ...[...new Set(billing.map((account) => account.vendor))]
        .filter(
          (vendor) => !config.panelLayout.blocks?.[`api:${vendor}`]?.hidden,
        )
        .map((vendor) => ({
          key: `api:${vendor}`,
          content: (
            <SourceBalances
              billing={billing.filter((account) => account.vendor === vendor)}
              layout={config.panelLayout.blocks?.[`api:${vendor}`]}
              light={light}
              Logo={Logo}
              onStats={onStats}
            />
          ),
        })),
    ],
    config.panelOrder ?? [],
  );
  const blockLines: (typeof blocks)[] = [];
  for (let i = 0; i < blocks.length; i++) {
    const block = blocks[i],
      next = blocks[i + 1];
    if (
      (contentWidth ?? config.panelWidth - 40) - density.section >= 460 &&
      next &&
      config.panelLayout.blocks?.[block.key]?.span === "half" &&
      config.panelLayout.blocks?.[next.key]?.span === "half"
    ) {
      blockLines.push([block, next]);
      i++;
    } else blockLines.push([block]);
  }
  // The listed sections show, in that order; anything unlisted stays out of
  // the panel — an empty list is a blank panel, not the default one.
  const wanted = config.panelSections ?? defaultHUDConfig.panelSections;
  const sections = inPanelOrder<{ key: string; content: ReactNode }>(
    [
      {
        key: "blocks",
        content:
          wanted.includes("blocks") &&
          config.quotaDetails &&
          blocks.length > 0 && (
          <div
            className="hud-source-blocks"
            style={{
              display: "flex",
              flexDirection: "column",
              gap: "var(--hud-section-gap)",
            }}
          >
            {blockLines.map((line, index) => (
              <div
                key={line.map((b) => b.key).join("+")}
                style={{
                  display: "grid",
                  gridTemplateColumns: line.length === 2 ? "1fr 1fr" : "1fr",
                  gap: density.section,
                  ...(index > 0 &&
                  config.panelLayout.style?.separators !== false
                    ? {
                        boxShadow: `inset 0 1px 0 ${colors.divider}`,
                        paddingTop: "var(--hud-divided-gap)",
                      }
                    : {}),
                }}
              >
                {line.map((block) => (
                  <div
                    key={block.key}
                    data-hud-block={block.key}
                    data-hud-half={line.length === 2 ? "true" : undefined}
                    style={{ minWidth: 0 }}
                  >
                    {block.content}
                  </div>
                ))}
              </div>
            ))}
          </div>
        ),
      },
      {
        key: "tokens",
        content: wanted.includes("tokens") &&
          config.tokenUsage && (
            <SourceTokenChart
              snapshot={snapshot}
              focusVendor={focusVendor}
              light={light}
              Logo={Logo}
              layout={config.panelLayout.tokens}
              active={active}
            />
          ),
      },
      {
        key: "sessions",
        content:
          wanted.includes("sessions") &&
          config.sessionList && (
          <SourceSessions
            snapshot={snapshot}
            focusVendor={focusVendor}
            light={light}
            Logo={Logo}
            onStats={onStats}
            layout={config.panelLayout.sessions}
            active={active}
          />
        ),
      },
    ],
    wanted,
  );
  const panel = (
    <div
      ref={panelElement}
      className="hud-source-hover-panel"
      data-density={config.layoutDensity}
      style={
        {
          "--hud-section-gap": `${density.section}px`,
          "--hud-block-gap": `${density.block}px`,
          "--hud-row-padding": `${density.row}px`,
          "--hud-session-gap": `${density.session}px`,
          "--hud-divided-gap": `${density.divided}px`,
          display: "flex",
          flexDirection: "column",
          gap: density.section,
          maxHeight,
          color: colors.text,
        } as CSSProperties
      }
    >
      <div
        data-hud-natural-scroll
        style={{
          overflowY: "auto",
          minHeight: 0,
          scrollbarWidth: "thin",
          flex: "1 1 auto",
        }}
      >
        <div
          className="hud-source-sections"
          style={{
            display: "flex",
            flexDirection: "column",
            gap: density.section,
          }}
        >
          {inlineAlert && (
            <div
              data-hud-section="inline-alert"
              style={
                config.panelLayout.style?.separators !== false
                  ? {
                      boxShadow: `inset 0 -1px 0 ${colors.divider}`,
                      paddingBottom: "var(--hud-divided-gap)",
                    }
                  : undefined
              }
            >
              {inlineAlert}
            </div>
          )}
          <RemoteSessionsBoard
            snapshot={snapshot}
            colors={colors}
            config={config}
          />
          <PendingRequestsBoard
            snapshot={snapshot}
            light={light}
            colors={colors}
            onDecide={onDecide}
          />
          {snapshot.indexing?.kind && (
            <div
              data-hud-section="indexing"
              style={
                config.panelLayout.style?.separators !== false
                  ? {
                      boxShadow: `inset 0 -1px 0 ${colors.divider}`,
                      paddingBottom: "var(--hud-divided-gap)",
                    }
                  : undefined
              }
            >
              <SourceIndexing snapshot={snapshot} light={light} />
            </div>
          )}
          {focusVendor && !vendorNames.length && (
            <strong style={{ fontSize: 13, fontWeight: 600 }}>
              {focusVendor}
            </strong>
          )}
          {sections.map((section) =>
            section.content ? (
              <div key={section.key} data-hud-section={section.key}>
                {section.content}
              </div>
            ) : null,
          )}
        </div>
      </div>
      {services?.draft?.unsaved && (
        <div
          style={{
            display: "flex",
            gap: 8,
            alignItems: "center",
            fontSize: 11,
            color: colors.secondary,
          }}
        >
          <div style={{ flex: 1, display: "grid", gap: 1, minWidth: 0 }}>
            <strong style={{ fontWeight: 600, color: colors.text }}>
              {draftTitle(services.draft!, snapshotLanguage(snapshot))}
            </strong>
            <FittingName
              {...draftNote(services.draft!, snapshotLanguage(snapshot))}
              style={{ fontSize: 10, color: colors.secondary }}
            />
          </div>
          {draftError && <span role="alert">{draftError}</span>}
          <Button
            size="xs"
            variant="ghost"
            onClick={() =>
              void services
                .draft!.discard()
                .catch((error) => setDraftError(String(error)))
            }
          >
            {snapshotLanguage(snapshot) === "zh" ? "丢弃" : "Discard"}
          </Button>
          <Button
            size="xs"
            onClick={() =>
              void services
                .draft!.save()
                .catch((error) => setDraftError(String(error)))
            }
          >
            {snapshotLanguage(snapshot) === "zh" ? "保存" : "Save"}
          </Button>
        </div>
      )}
      {services?.draft?.heldUntil && (
        <Button size="xs" variant="ghost" onClick={services.draft.release}>
          {snapshotLanguage(snapshot) === "zh" ? "完成编辑" : "Finish editing"}
        </Button>
      )}
      <footer
        data-hud-footer
        className="hud-source-footer"
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
          flex: "0 0 auto",
          paddingTop: 6,
          boxShadow: `inset 0 1px 0 ${colors.divider}`,
          color: colors.secondary,
        }}
      >
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={snapshotLanguage(snapshot) === "zh" ? "设置" : "Settings"}
          title={snapshotLanguage(snapshot) === "zh" ? "设置" : "Settings"}
          disabled={!onConfigure}
          onClick={onConfigure}
          style={{
            width: 28,
            height: 28,
            minHeight: 28,
            padding: 0,
            borderRadius: 0,
            color: colors.secondary,
            background: "transparent",
          }}
        >
          <Settings style={{ width: 14, height: 14 }} strokeWidth={1.7} />
        </Button>
        <div style={{ display: "flex", alignItems: "center" }}>
          {update && (
            // It arrives as the HUD's other changes do: it grows in, or with
            // reduced motion only fades in.
            <motion.span
              key={update.version}
              initial={{ opacity: 0, scale: 0.6 }}
              animate={{ opacity: 1, scale: 1 }}
              transition={{ duration: 0.22, ease: "easeOut" }}
              style={{ display: "inline-flex" }}
            >
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={
                  snapshotLanguage(snapshot) === "zh"
                    ? "发现新版本"
                    : "New version available"
                }
                title={
                  snapshotLanguage(snapshot) === "zh"
                    ? `更新至 ${update.version}`
                    : `Update to ${update.version}`
                }
                disabled={!onUpdate}
                onClick={onUpdate}
                style={{
                  width: 28,
                  height: 28,
                  minHeight: 28,
                  padding: 0,
                  borderRadius: 0,
                  color: colors.accent,
                  background: "transparent",
                }}
              >
                <CircleArrowDown
                  style={{ width: 14, height: 14 }}
                  strokeWidth={1.7}
                />
              </Button>
            </motion.span>
          )}
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={
              snapshotLanguage(snapshot) === "zh"
                ? "用量统计"
                : "Usage statistics"
            }
            title={
              snapshotLanguage(snapshot) === "zh"
                ? "用量统计"
                : "Usage statistics"
            }
            disabled={!onStats}
            onClick={onStats}
            style={{
              width: 28,
              height: 28,
              minHeight: 28,
              padding: 0,
              borderRadius: 0,
              color: colors.secondary,
              background: "transparent",
            }}
          >
            <ChartNoAxesColumnIncreasing
              style={{ width: 14, height: 14 }}
              strokeWidth={1.7}
            />
          </Button>
        </div>
      </footer>
    </div>
  );
  // The panel's charts move as the HUD does: not at all when animations are
  // off (the setting, a picture) or the system asks for reduced motion.
  return (
    <MotionConfig
      reducedMotion={
        config.animations && snapshot.settings.animations ? "user" : "always"
      }
    >
      {panel}
    </MotionConfig>
  );
});

type DraftState = NonNullable<
  NonNullable<ReturnType<typeof usePanelServices>>["draft"]
>;

/** What waits (Open's PanelDraftBar.title): a layout, with or without a
 * style, a style alone, or the app's settings alone. */
/** The remote sessions at a glance: who runs, who idles, and who waits for
 * an answer — the wait a waiting card on the island carries. */
function RemoteSessionsBoard({
  snapshot,
  colors,
  config,
}: {
  snapshot: Snapshot;
  colors: ReturnType<typeof palette>;
  config: HUDConfig;
}) {
  const remote = useMemo(
    () =>
      snapshot.sessions.filter(
        (session) => session.source === "claude-remote",
      ),
    [snapshot.sessions],
  );
  const answered = useMemo(
    () =>
      new Set(
        snapshot.approvals
          .filter((request) => request.source === "claude-remote")
          .map((request) => request.sessionId),
      ),
    [snapshot.approvals],
  );
  // The names carry the user's own taxonomy, so the board folds itself into
  // their groups; a single flat group renders exactly as it did ungrouped.
  const groups = useMemo(
    () =>
      groupRemoteSessions(remote, (session) =>
        session.state === "waiting" || answered.has(session.id)
          ? 0
          : session.state === "running"
            ? 1
            : 2,
      ),
    [remote, answered],
  );
  // A fold survives the panel closing and the app restarting: the webview's
  // localStorage keeps it, so the board reopens the way it was left.
  const [folded, setFolded] = useState<Record<string, boolean>>(() => {
    try {
      return JSON.parse(
        localStorage.getItem("agent-hud.remote-groups.folded") ?? "{}",
      );
    } catch {
      return {};
    }
  });
  const fold = (key: string, foldedNow: boolean) =>
    setFolded((f) => {
      const next = { ...f, [key]: foldedNow };
      try {
        localStorage.setItem(
          "agent-hud.remote-groups.folded",
          JSON.stringify(next),
        );
      } catch {}
      return next;
    });
  // An open detail card is browsing, not a preference: it lives with the
  // panel and closes with it.
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const toggleRow = (id: string) =>
    setExpanded((e) => ({ ...e, [id]: !e[id] }));
  const zh = snapshotLanguage(snapshot) === "zh";
  if (remote.length === 0) return null;
  const isWaiting = (session: Session) =>
    session.state === "waiting" || answered.has(session.id);
  // The remote list's own order: who waits first, then who runs, then the
  // finished — each group newest first, like claude's own resume list.
  const rank = (session: Session) =>
    isWaiting(session) ? 0 : session.state === "running" ? 1 : 2;
  const fresh = (a: Session, b: Session) =>
    Date.parse(b.lastMessageAt ?? b.updatedAt) -
    Date.parse(a.lastMessageAt ?? a.updatedAt);
  const ordered = [...remote].sort((a, b) => rank(a) - rank(b) || fresh(a, b));
  const waitingCount = ordered.filter(isWaiting).length;
  const runningCount = ordered.filter(
    (session) => !isWaiting(session) && session.state === "running",
  ).length;
  // A bare "2m/3h/9d", the style claude's own list prints next to a name.
  const ago = (session: Session) => {
    const now = Date.parse(snapshot.generatedAt);
    const at = Date.parse(session.lastMessageAt ?? session.updatedAt);
    if (!Number.isFinite(at) || !Number.isFinite(now)) return "";
    return age(Math.max(0, (now - at) / 1000), zh ? "zh" : "en")
      .replace(" ago", "")
      .replace(" 前", "");
  };
  // An absolute stamp for the detail card: today reads as a clock time, and
  // anything older keeps its date.
  const when = (iso: string | null | undefined) => {
    if (!iso) return "";
    const at = Date.parse(iso);
    if (!Number.isFinite(at)) return "";
    const atDate = new Date(at);
    const clock = `${String(atDate.getHours()).padStart(2, "0")}:${String(atDate.getMinutes()).padStart(2, "0")}`;
    const now = Date.parse(snapshot.generatedAt) || Date.now();
    const midnight = new Date(now);
    midnight.setHours(0, 0, 0, 0);
    if (at >= midnight.getTime())
      return clock;
    if (at >= midnight.getTime() - 86_400_000)
      return zh ? `昨天 ${clock}` : `yesterday ${clock}`;
    return `${atDate.getMonth() + 1}/${atDate.getDate()} ${clock}`;
  };
  // How long a turn ran, or has been running.
  const span = (fromIso: string | null, toIso: string | null, live: boolean) => {
    const from = fromIso ? Date.parse(fromIso) : NaN;
    const to = live ? Date.parse(snapshot.generatedAt) : toIso ? Date.parse(toIso) : NaN;
    if (!Number.isFinite(from) || !Number.isFinite(to)) return "";
    const seconds = Math.max(0, Math.round((to - from) / 1000));
    if (seconds < 90) return `${seconds}s`;
    const minutes = Math.round(seconds / 60);
    if (minutes < 90) return `${minutes}m`;
    const hours = Math.floor(minutes / 60);
    return hours >= 36 ? `${Math.round(hours / 24)}d` : `${hours}h ${minutes % 60}m`;
  };
  return (
    <div
      data-hud-section="remote-sessions"
      style={
        config.panelLayout.style?.separators !== false
          ? {
              display: "grid",
              gap: "var(--hud-session-gap, 6px)",
              boxShadow: `inset 0 -1px 0 ${colors.divider}`,
              paddingBottom: "var(--hud-divided-gap)",
            }
          : { display: "grid", gap: "var(--hud-session-gap, 6px)" }
      }
    >
      <div style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
        <strong style={{ fontSize: 13, fontWeight: 600, color: colors.text }}>
          {zh ? "远程会话" : "Remote sessions"}
        </strong>
        <span
          style={{
            fontSize: 10,
            fontWeight: 600,
            fontVariantNumeric: "tabular-nums",
            padding: "2px 6px",
            borderRadius: 999,
            color: waitingCount > 0 ? colors.warningText : colors.secondary,
            background:
              waitingCount > 0 ? "rgba(233, 161, 109, 0.16)" : "transparent",
          }}
        >
          {zh
            ? `${waitingCount} 等待 · ${runningCount} 运行 · ${ordered.length - waitingCount - runningCount} 完成`
            : `${waitingCount} waiting · ${runningCount} running · ${ordered.length - waitingCount - runningCount} done`}
        </span>
      </div>
      {(() => {
        const row = (session: Session) => {
          const waiting = isWaiting(session);
          const running = !waiting && session.state === "running";
          const message = session.liveFacts?.message?.trim() ?? "";
          // A finished row gone quiet for over half an hour steps back —
          // the active few stand out of the long tail.
          const now = Date.parse(snapshot.generatedAt);
          const at = Date.parse(session.lastMessageAt ?? session.updatedAt);
          const stale =
            !waiting &&
            !running &&
            Number.isFinite(now) &&
            Number.isFinite(at) &&
            now - at > 30 * 60_000;
          const open = Boolean(expanded[session.id]);
          const facts = session.liveFacts ?? null;
          // The fork parent sits in this same list; its id there carries the
          // source prefix this row's parentId drops.
          const parentTitle = session.parentId
            ? (remote.find((item) => item.id.endsWith(session.parentId!))
                ?.title ?? null)
            : null;
          return (
            <div key={session.id} style={{ display: "grid" }}>
              <div
                onClick={() => toggleRow(session.id)}
                style={{
                  display: "flex",
                  alignItems: "baseline",
                  gap: 8,
                  fontSize: 12,
                  lineHeight: "16px",
                  opacity: stale ? 0.45 : 1,
                  cursor: "pointer",
                }}
              >
                <span
                  aria-hidden
                  style={{
                    width: 6,
                    height: 6,
                    borderRadius: 999,
                    flex: "0 0 auto",
                    alignSelf: "center",
                    background: waiting
                      ? colors.warning
                      : running
                        ? colors.ok
                        : colors.tertiary,
                  }}
                />
                <span
                  title={session.title || session.id}
                  style={{
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                    flexShrink: 0,
                    maxWidth: 180,
                    color: waiting ? colors.warningText : colors.text,
                    fontWeight: waiting ? 600 : 400,
                  }}
                >
                  {session.parentId ? "⑂ " : ""}
                  {session.title || session.id}
                </span>
                <span
                  title={message}
                  style={{
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                    flex: 1,
                    minWidth: 32,
                    fontSize: 11,
                    color: colors.secondary,
                  }}
                >
                  {message}
                </span>
                <span
                  style={{
                    flexShrink: 0,
                    fontSize: 11,
                    fontVariantNumeric: "tabular-nums",
                    whiteSpace: "nowrap",
                    color: colors.tertiary,
                  }}
                >
                  {ago(session)}
                </span>
              </div>
              {open && (
                <div
                  style={{
                    margin: "3px 0 4px 20px",
                    padding: "8px 10px",
                    borderRadius: 8,
                    background: "rgba(128, 128, 128, 0.08)",
                    display: "grid",
                    gap: 5,
                    fontSize: 11,
                    lineHeight: "15px",
                    color: colors.secondary,
                  }}
                >
                  <div style={{ display: "flex", gap: 10, alignItems: "baseline" }}>
                    <span
                      style={{
                        fontWeight: 650,
                        color: waiting
                          ? colors.warningText
                          : running
                            ? colors.okText
                            : colors.secondary,
                      }}
                    >
                      {waiting
                        ? zh
                          ? "等待输入"
                          : "waiting"
                        : running
                          ? zh
                            ? "运行中"
                            : "running"
                          : facts?.outcome === "failed"
                            ? zh
                              ? "失败"
                              : "failed"
                            : facts?.outcome === "interrupted"
                              ? zh
                                ? "已中断"
                                : "interrupted"
                              : zh
                                ? "已完成"
                                : "done"}
                    </span>
                    {facts?.turnStartedAt && (
                      <span style={{ fontVariantNumeric: "tabular-nums" }}>
                        {zh ? "本轮" : "turn"} {when(facts.turnStartedAt)}
                        {facts.turnStartedAt &&
                          ` · ${span(facts.turnStartedAt, facts.endedAt, running)}`}
                      </span>
                    )}
                  </div>
                  <div style={{ fontVariantNumeric: "tabular-nums" }}>
                    {zh ? "最后活动" : "last activity"}{" "}
                    {when(session.lastMessageAt ?? session.updatedAt)}
                    {session.cwd ? ` · ${session.cwd}` : ""}
                  </div>
                  {parentTitle && (
                    <div>
                      ⑂ {zh ? "源自" : "forked from"} {parentTitle}
                    </div>
                  )}
                  {message && (
                    <div
                      style={{
                        whiteSpace: "pre-wrap",
                        color: colors.text,
                        fontSize: 12,
                        lineHeight: "17px",
                      }}
                    >
                      {message}
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        };
        // One group is no grouping: the board stays a plain list.
        if (groups.length <= 1)
          return (groups[0]?.sessions ?? []).map(row);
        return groups.map((group) => {
          const open = !folded[group.key];
          const live = group.sessions.some(isWaiting)
            ? colors.warning
            : group.sessions.some((session) => session.state === "running")
              ? colors.ok
              : null;
          const label =
            group.key === REMOTE_OTHER_GROUP
              ? zh
                ? "其他"
                : "Other"
              : group.key;
          return (
            <div
              key={group.key}
              style={{ display: "grid", gap: "var(--hud-session-gap, 6px)" }}
            >
              <button
                type="button"
                onClick={() => fold(group.key, open)}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 6,
                  border: 0,
                  padding: 0,
                  background: "transparent",
                  cursor: "pointer",
                  textAlign: "left",
                  fontSize: 12.5,
                  fontWeight: 650,
                  letterSpacing: "0.02em",
                  color: colors.warningText,
                }}
              >
                <span aria-hidden style={{ width: 8, flexShrink: 0 }}>
                  {open ? "▾" : "▸"}
                </span>
                {live && (
                  <span
                    aria-hidden
                    style={{
                      width: 5,
                      height: 5,
                      borderRadius: 999,
                      flexShrink: 0,
                      background: live,
                    }}
                  />
                )}
                <span>{label}</span>
                <span
                  style={{
                    marginLeft: "auto",
                    fontWeight: 400,
                    fontVariantNumeric: "tabular-nums",
                    color: colors.tertiary,
                  }}
                >
                  {group.sessions.length}
                </span>
              </button>
              {open && group.sessions.map(row)}
            </div>
          );
        });
      })()}
    </div>
  );
}

/** The island's pending answers, listed: every request that waits, local or
 * remote, with the source it came from. A row opens its full card inline —
 * the same content the island card shows — where it can be answered. */
function PendingRequestsBoard({
  snapshot,
  light,
  colors,
  onDecide,
}: {
  snapshot: Snapshot;
  light: boolean;
  colors: ReturnType<typeof palette>;
  onDecide?: (id: string, decision: RequestDecision) => void | Promise<void>;
}) {
  const pending = snapshot.approvals;
  const zh = snapshotLanguage(snapshot) === "zh";
  const [openId, setOpenId] = useState<string | null>(null);
  // A card that was answered elsewhere leaves the list; close the pane then.
  const opened = pending.find((request) => request.id === openId);
  if (pending.length === 0) return null;
  return (
    <div
      data-hud-section="pending-requests"
      style={{
        display: "grid",
        gap: "var(--hud-session-gap, 6px)",
        boxShadow: `inset 0 -1px 0 ${colors.divider}`,
        paddingBottom: "var(--hud-divided-gap)",
      }}
    >
      <div style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
        <strong style={{ fontSize: 13, fontWeight: 600, color: colors.text }}>
          {zh ? "待处理" : "Waiting"}
        </strong>
        <span
          style={{
            marginLeft: "auto",
            fontSize: 10,
            fontWeight: 600,
            fontVariantNumeric: "tabular-nums",
            color: colors.warningText,
            padding: "2px 6px",
            borderRadius: 999,
            background: "rgba(233, 161, 109, 0.16)",
          }}
        >
          {pending.length}
        </span>
      </div>
      {pending.map((request) => {
        const question = (request.questions ?? [])[0];
        const label =
          request.title?.trim() ||
          question?.header?.trim() ||
          question?.question?.trim() ||
          clientDisplayName(request.source);
        const open = request.id === openId;
        return (
          <div key={request.id} style={{ display: "grid", gap: 6 }}>
            <button
              type="button"
              onClick={() => setOpenId(open ? null : request.id)}
              style={{
                display: "flex",
                alignItems: "baseline",
                gap: 8,
                width: "100%",
                fontSize: 12,
                lineHeight: "16px",
                padding: 0,
                border: 0,
                background: "transparent",
                cursor: "pointer",
                textAlign: "left",
                font: "inherit",
                color: "inherit",
              }}
            >
              <span
                aria-hidden
                style={{
                  width: 6,
                  height: 6,
                  borderRadius: 999,
                  flex: "0 0 auto",
                  alignSelf: "center",
                  background: colors.warning,
                }}
              />
              <span
                style={{
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                  color: colors.warningText,
                  fontWeight: 600,
                }}
              >
                {label}
              </span>
              <span
                style={{
                  marginLeft: "auto",
                  fontSize: 11,
                  whiteSpace: "nowrap",
                  color: open ? colors.text : colors.secondary,
                }}
              >
                {clientDisplayName(request.source)}
              </span>
            </button>
            {open && opened && (
              <ApprovalContent
                requests={pending}
                selected={opened}
                select={(id) => setOpenId(id)}
                decide={onDecide}
                light={light}
                language={snapshotLanguage(snapshot)}
              />
            )}
          </div>
        );
      })}
    </div>
  );
}

function draftTitle(draft: DraftState, language: "en" | "zh") {
  const zh = language === "zh";
  if (draft.layout) return zh ? "布局未保存" : "Unsaved layout";
  if (draft.style) return zh ? "HUD 样式未保存" : "Unsaved HUD style";
  return zh ? "应用设置未保存" : "Unsaved app settings";
}

/** When the draft goes, and whose it is (Open's PanelDraftBar.note): this
 * display's alone, or with the app's settings, which are every display's;
 * `short` where `name` does not fit. */
function draftNote(draft: DraftState, language: "en" | "zh") {
  const zh = language === "zh";
  const restart = zh ? "重启后丢弃" : "Dropped on restart";
  if (draft.app) {
    if (!draft.layout && !draft.style) return { name: restart, short: restart };
    return zh
      ? { name: "含应用设置，重启后丢弃", short: "含应用设置" }
      : {
          name: "With app settings, dropped on restart",
          short: "With app settings",
        };
  }
  if (draft.layout === "shared") return { name: restart, short: restart };
  return zh
    ? { name: "仅此显示器，重启后丢弃", short: "仅此显示器" }
    : {
        name: "This display only, dropped on restart",
        short: "This display only",
      };
}
