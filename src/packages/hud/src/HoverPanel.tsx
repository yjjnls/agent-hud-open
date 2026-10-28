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
import type { Quota, Snapshot } from "@agent-hud/protocol";
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
import { clientDisplayName } from "./IslandNews";
import { snapshotLanguage } from "@agent-hud/ui/format";

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
            light={light}
            colors={colors}
            config={config}
          />
          <PendingRequestsBoard snapshot={snapshot} light={light} colors={colors} />
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
  light,
  colors,
  config,
}: {
  snapshot: Snapshot;
  light: boolean;
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
  const zh = snapshotLanguage(snapshot) === "zh";
  if (remote.length === 0) return null;
  const waiting = remote.filter(
    (session) => session.state === "waiting" || answered.has(session.id),
  ).length;
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
        {waiting > 0 && (
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
            {zh ? `${waiting} 个等待回答` : `${waiting} waiting`}
          </span>
        )}
      </div>
      {remote.map((session) => {
        const isWaiting =
          session.state === "waiting" || answered.has(session.id);
        const running = session.state === "running";
        const label = isWaiting
          ? zh
            ? "等待回答"
            : "Waiting"
          : running
            ? zh
              ? "运行中"
              : "Running"
            : zh
              ? "空闲"
              : "Idle";
        const tint = isWaiting
          ? colors.warningText
          : running
            ? colors.text
            : colors.secondary;
        return (
          <div
            key={session.id}
            style={{
              display: "flex",
              alignItems: "baseline",
              gap: 8,
              fontSize: 12,
              lineHeight: "16px",
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
                background: isWaiting
                  ? colors.warning
                  : running
                    ? colors.ok
                    : colors.tertiary,
              }}
            />
            <span
              style={{
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                color: isWaiting ? colors.warningText : colors.text,
                fontWeight: isWaiting ? 600 : 400,
              }}
            >
              {session.title || session.id}
            </span>
            <span
              style={{
                marginLeft: "auto",
                fontSize: 11,
                whiteSpace: "nowrap",
                color: tint,
              }}
            >
              {label}
            </span>
          </div>
        );
      })}
    </div>
  );
}

/** The island's pending answers, listed: every request that waits, local or
 * remote, with the source it came from. The island card is where an answer
 * goes; this board is where they are all visible at once. */
function PendingRequestsBoard({
  snapshot,
  light,
  colors,
}: {
  snapshot: Snapshot;
  light: boolean;
  colors: ReturnType<typeof palette>;
}) {
  const pending = snapshot.approvals;
  const zh = snapshotLanguage(snapshot) === "zh";
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
        return (
          <div
            key={request.id}
            style={{
              display: "flex",
              alignItems: "baseline",
              gap: 8,
              fontSize: 12,
              lineHeight: "16px",
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
                color: colors.secondary,
              }}
            >
              {clientDisplayName(request.source)}
            </span>
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
