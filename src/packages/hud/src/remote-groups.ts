import type { Session } from "@agent-hud/protocol";

/** The group a session's own name sorts into. The remote list is the user's
 * own resume list, and their names already carry the taxonomy: a Chinese
 * prefix ("调研-", "问题排查-") is the folder they meant, a numbered host
 * name ("deepinfer2-ui", "sandbox3-user-quota") belongs to its base, and a
 * name standing alone groups with nothing. */
export function remoteGroupKey(title: string): string {
  const name = title.trim();
  const head = name.split(/[\s-]+/)[0] ?? name;
  // A pure-Chinese head is the user's own category word: keep it whole.
  if (/^[一-鿿]+$/.test(head)) return head;
  // Otherwise the leading run of letters is the base — "deepinfer2" is
  // deepinfer, "opensandbox" is its own word and stays its own.
  const base = /^[a-zA-Z]+/.exec(head)?.[0]?.toLowerCase();
  return base || name;
}

/** A group's sessions, the board's own order inside: who waits, then who
 * runs, then the finished, each newest first. */
export interface RemoteGroup {
  key: string;
  sessions: Session[];
}

export const REMOTE_OTHER_GROUP = "other";

export function groupRemoteSessions(
  sessions: Session[],
  rank: (session: Session) => number,
): RemoteGroup[] {
  const stamp = (session: Session) =>
    Date.parse(session.lastMessageAt ?? session.updatedAt) || 0;
  const order = (list: Session[]) =>
    list.sort((a, b) => rank(a) - rank(b) || stamp(b) - stamp(a));
  const grouped = new Map<string, Session[]>();
  for (const session of sessions) {
    const key = remoteGroupKey(session.title || session.id);
    const list = grouped.get(key);
    if (list) list.push(session);
    else grouped.set(key, [session]);
  }
  // A group of one says nothing the row itself doesn't; fold it away.
  const groups: RemoteGroup[] = [];
  const lone: Session[] = [];
  for (const [key, list] of grouped)
    if (list.length > 1) groups.push({ key, sessions: order(list) });
    else lone.push(list[0]);
  if (lone.length) groups.push({ key: REMOTE_OTHER_GROUP, sessions: order(lone) });
  // Live groups lead: the least-active rank first, then the freshest group.
  return groups.sort((a, b) => {
    if (a.key === REMOTE_OTHER_GROUP) return 1;
    if (b.key === REMOTE_OTHER_GROUP) return -1;
    const top = (group: RemoteGroup) => Math.min(...group.sessions.map(rank));
    const newest = (group: RemoteGroup) => Math.max(...group.sessions.map(stamp));
    return top(a) - top(b) || newest(b) - newest(a) || a.key.localeCompare(b.key);
  });
}
