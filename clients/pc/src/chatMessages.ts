/**
 * Chat transcript → bubbles.
 *
 * `getAgentTranscriptTail` (all gateways) returns:
 * `{ entries: [{ id, role, text, seq }], agentId }`.
 * Live `hub:*` events are not inputs here, so turn_finished previews
 * cannot double-show next to the transcript body.
 */

export interface ChatMessage {
  role: string;
  content: string;
  /** Transcript entry id when the gateway sent one. */
  id?: string;
  /** Local send not yet present in the tail. */
  pending?: boolean;
}

export interface PendingUserSend {
  localId: string;
  content: string;
  /** User-message ids with this content already in the tail at send time. */
  seenIds: string[];
  /** Id-less user messages with this content already in the tail at send time. */
  unidentifiedCount: number;
}

interface Draft extends ChatMessage {
  seq?: number;
}

/** FNV-1a 32-bit. Stable across runs; used only when an entry has no id. */
export function hashRoleContent(role: string, content: string): string {
  let h = 0x811c9dc5;
  const s = `${role}\0${content}`;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return (h >>> 0).toString(16).padStart(8, "0");
}

/** Id wins. Otherwise a stable hash of role + content. */
export function messageDedupeKey(m: Pick<ChatMessage, "id" | "role" | "content">): string {
  if (m.id) return `id:${m.id}`;
  return `hash:${hashRoleContent(m.role, m.content)}`;
}

/**
 * Collapse duplicates:
 * - same transcript id anywhere in the list
 * - consecutive id-less messages with the same role + content
 * Non-consecutive identical bodies with different ids both stay
 * (the user can say the same thing twice).
 */
export function dedupeMessages(messages: ChatMessage[]): ChatMessage[] {
  const seenIds = new Set<string>();
  const out: ChatMessage[] = [];
  for (const m of messages) {
    if (m.id) {
      if (seenIds.has(m.id)) continue;
      seenIds.add(m.id);
      out.push(m);
      continue;
    }
    const prev = out[out.length - 1];
    if (prev && !prev.id && messageDedupeKey(prev) === messageDedupeKey(m)) continue;
    out.push(m);
  }
  return out;
}

function asRecord(v: unknown): Record<string, unknown> | null {
  if (!v || typeof v !== "object" || Array.isArray(v)) return null;
  return v as Record<string, unknown>;
}

function draftFromEntry(v: unknown): Draft | null {
  const o = asRecord(v);
  if (!o) return null;
  const role = typeof o.role === "string" ? o.role.trim() : "";
  if (!role) return null;
  // TranscriptEntry.text is the body. Do not read hub event `preview`.
  if (typeof o.text !== "string") return null;
  const id = typeof o.id === "string" && o.id.length > 0 ? o.id : undefined;
  const seq = typeof o.seq === "number" && Number.isFinite(o.seq) ? o.seq : undefined;
  return { role, content: o.text, id, seq };
}

function entryList(raw: unknown): unknown[] | null {
  if (Array.isArray(raw)) return raw;
  const o = asRecord(raw);
  if (!o) return null;
  if (Array.isArray(o.entries)) return o.entries;
  return null;
}

/**
 * Parse a `getAgentTranscriptTail` result into chat messages, oldest first.
 * Missing `text` (for example a `hub:turn_finished` object) is skipped.
 * A JSON string is not a tail result and yields nothing.
 */
export function parseTranscriptMessages(raw: unknown): ChatMessage[] {
  const list = entryList(raw);
  if (!list) return [];
  const drafts: Draft[] = [];
  for (const item of list) {
    const d = draftFromEntry(item);
    if (d) drafts.push(d);
  }
  const anySeq = drafts.some((d) => d.seq !== undefined);
  if (anySeq) {
    const indexed = drafts.map((d, i) => ({ d, i }));
    indexed.sort((a, b) => {
      const as = a.d.seq;
      const bs = b.d.seq;
      if (as === undefined && bs === undefined) return a.i - b.i;
      if (as === undefined) return 1;
      if (bs === undefined) return -1;
      if (as !== bs) return as - bs;
      return a.i - b.i;
    });
    return indexed.map(({ d }) => ({ role: d.role, content: d.content, id: d.id }));
  }
  return drafts.map((d) => ({ role: d.role, content: d.content, id: d.id }));
}

/** Snapshot of user bubbles already showing this text, for optimistic send. */
export function snapshotUserContent(
  messages: ChatMessage[],
  content: string,
): { seenIds: string[]; unidentifiedCount: number } {
  const seenIds: string[] = [];
  let unidentifiedCount = 0;
  for (const m of dedupeMessages(messages)) {
    if (m.role !== "user" || m.content !== content) continue;
    if (m.id) seenIds.push(m.id);
    else unidentifiedCount += 1;
  }
  return { seenIds, unidentifiedCount };
}

/**
 * Append local user sends until the tail contains a new matching user entry.
 * Matching prefers a transcript id that was not in `seenIds` (works when the
 * tail window is capped and older copies scroll off). Id-less tails fall
 * back to a count increase.
 */
export function applyOptimisticUsers(
  tail: ChatMessage[],
  pending: PendingUserSend[],
): { messages: ChatMessage[]; remaining: PendingUserSend[] } {
  const base = dedupeMessages(tail);
  const claimed = new Set<string>();
  const usedIdLess = new Map<string, number>();
  const remaining: PendingUserSend[] = [];
  const extras: ChatMessage[] = [];

  for (const p of pending) {
    const match = base.find(
      (m) =>
        m.role === "user" &&
        m.content === p.content &&
        !!m.id &&
        !p.seenIds.includes(m.id) &&
        !claimed.has(m.id),
    );
    if (match?.id) {
      claimed.add(match.id);
      continue;
    }
    const idLess = base.filter(
      (m) => m.role === "user" && m.content === p.content && !m.id,
    ).length;
    const used = usedIdLess.get(p.content) ?? 0;
    if (idLess > p.unidentifiedCount + used) {
      usedIdLess.set(p.content, used + 1);
      continue;
    }
    remaining.push(p);
    extras.push({
      role: "user",
      content: p.content,
      id: p.localId,
      pending: true,
    });
  }

  return { messages: dedupeMessages([...base, ...extras]), remaining };
}
