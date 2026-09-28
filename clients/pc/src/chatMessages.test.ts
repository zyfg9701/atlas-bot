import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  applyOptimisticUsers,
  dedupeMessages,
  hashRoleContent,
  messageDedupeKey,
  parseTranscriptMessages,
  snapshotUserContent,
  type ChatMessage,
  type PendingUserSend,
} from "./chatMessages";

/** Shape returned by every gateway's getAgentTranscriptTail. */
const TAIL = {
  agentId: "agt_2",
  entries: [
    {
      id: "msg_seed_agt_2",
      role: "assistant",
      text: "agent Scout ready",
      seq: 1,
    },
    {
      id: "msg_u_2",
      role: "user",
      text: "你能做什么",
      seq: 2,
    },
    {
      id: "msg_a_3",
      role: "assistant",
      text: "我可以：\n\n- **读代码**\n- 修 bug",
      seq: 3,
    },
    {
      id: "msg_sys_4",
      role: "system",
      text: "run interrupted",
      seq: 4,
    },
  ],
};

describe("parseTranscriptMessages", () => {
  it("reads entries[].text/role/id in seq order", () => {
    const shuffled = {
      agentId: "agt_2",
      entries: [...TAIL.entries].reverse(),
    };
    const msgs = parseTranscriptMessages(shuffled);
    assert.deepEqual(
      msgs.map((m) => m.id),
      ["msg_seed_agt_2", "msg_u_2", "msg_a_3", "msg_sys_4"],
    );
    assert.equal(msgs[2].role, "assistant");
    assert.equal(msgs[2].content, "我可以：\n\n- **读代码**\n- 修 bug");
  });

  it("accepts a bare entries array", () => {
    const msgs = parseTranscriptMessages(TAIL.entries);
    assert.equal(msgs.length, 4);
    assert.equal(msgs[1].content, "你能做什么");
  });

  it("skips hub event objects that have preview but no text", () => {
    const msgs = parseTranscriptMessages({
      entries: [
        {
          channel: "hub:turn_finished",
          event: { preview: "我可以：\n\n- **读代码**\n- 修 bug" },
          seq: 9,
        },
        { id: "msg_a_3", role: "assistant", text: "real body", seq: 3 },
      ],
    });
    assert.deepEqual(msgs, [{ role: "assistant", content: "real body", id: "msg_a_3" }]);
  });

  it("returns nothing for null, strings, and objects without entries", () => {
    assert.deepEqual(parseTranscriptMessages(null), []);
    assert.deepEqual(parseTranscriptMessages("—— transcript ——"), []);
    assert.deepEqual(parseTranscriptMessages({ agentId: "agt_1", preview: "x" }), []);
  });

  it("keeps input order when seq is absent", () => {
    const msgs = parseTranscriptMessages([
      { id: "b", role: "assistant", text: "second" },
      { id: "a", role: "user", text: "first" },
    ]);
    assert.deepEqual(
      msgs.map((m) => m.id),
      ["b", "a"],
    );
  });
});

describe("dedupeMessages", () => {
  it("collapses the same transcript id even when not adjacent", () => {
    const dup: ChatMessage[] = [
      { id: "msg_a_3", role: "assistant", content: "hello" },
      { id: "msg_u_9", role: "user", content: "next" },
      { id: "msg_a_3", role: "assistant", content: "hello" },
    ];
    const out = dedupeMessages(dup);
    assert.equal(out.length, 2);
    assert.equal(out[0].id, "msg_a_3");
    assert.equal(out[1].id, "msg_u_9");
  });

  it("collapses consecutive id-less copies and keeps a later repeat", () => {
    const msgs: ChatMessage[] = [
      { role: "assistant", content: "same" },
      { role: "assistant", content: "same" },
      { role: "user", content: "ok" },
      { role: "assistant", content: "same" },
    ];
    const out = dedupeMessages(msgs);
    assert.equal(out.length, 3);
    assert.equal(messageDedupeKey(out[0]), `hash:${hashRoleContent("assistant", "same")}`);
    assert.notEqual(messageDedupeKey(out[0]), messageDedupeKey(out[1]));
  });

  it("keeps two identical bodies when the gateway assigned different ids", () => {
    const out = dedupeMessages([
      { id: "msg_u_2", role: "user", content: "ping" },
      { id: "msg_a_3", role: "assistant", content: "pong" },
      { id: "msg_u_4", role: "user", content: "ping" },
    ]);
    assert.equal(out.length, 3);
  });

  it("hash is stable for the same role and content", () => {
    assert.equal(hashRoleContent("assistant", "a\nb"), hashRoleContent("assistant", "a\nb"));
    assert.notEqual(hashRoleContent("user", "a"), hashRoleContent("assistant", "a"));
  });
});

describe("applyOptimisticUsers", () => {
  it("shows a local user bubble until a new tail id arrives", () => {
    const before = parseTranscriptMessages(TAIL);
    const snap = snapshotUserContent(before, "你能做什么");
    const pending: PendingUserSend[] = [
      { localId: "local-1", content: "再问一次", seenIds: [], unidentifiedCount: 0 },
    ];
    // Different text: snapshot of the new prompt is empty.
    assert.deepEqual(snapshotUserContent(before, "再问一次"), {
      seenIds: [],
      unidentifiedCount: 0,
    });
    const waiting = applyOptimisticUsers(before, pending);
    assert.equal(waiting.remaining.length, 1);
    assert.equal(waiting.messages.at(-1)?.pending, true);
    assert.equal(waiting.messages.at(-1)?.content, "再问一次");

    const after = parseTranscriptMessages({
      entries: [
        ...TAIL.entries,
        { id: "msg_u_5", role: "user", text: "再问一次", seq: 5 },
        { id: "msg_a_6", role: "assistant", text: "**好**", seq: 6 },
      ],
    });
    const settled = applyOptimisticUsers(after, waiting.remaining);
    assert.equal(settled.remaining.length, 0);
    const users = settled.messages.filter((m) => m.role === "user");
    assert.equal(users.length, 2);
    assert.equal(users.filter((m) => m.pending).length, 0);
    assert.equal(settled.messages.filter((m) => m.content === "**好**").length, 1);
  });

  it("does not treat an older copy of the same text as the new send", () => {
    const tail = parseTranscriptMessages({
      entries: [{ id: "msg_u_2", role: "user", text: "ping", seq: 2 }],
    });
    const snap = snapshotUserContent(tail, "ping");
    assert.deepEqual(snap.seenIds, ["msg_u_2"]);
    const pending: PendingUserSend[] = [
      {
        localId: "local-2",
        content: "ping",
        seenIds: snap.seenIds,
        unidentifiedCount: snap.unidentifiedCount,
      },
    ];
    const still = applyOptimisticUsers(tail, pending);
    assert.equal(still.remaining.length, 1);
    assert.equal(still.messages.filter((m) => m.content === "ping").length, 2);

    const grown = parseTranscriptMessages({
      entries: [
        { id: "msg_u_2", role: "user", text: "ping", seq: 2 },
        { id: "msg_u_8", role: "user", text: "ping", seq: 8 },
      ],
    });
    const done = applyOptimisticUsers(grown, still.remaining);
    assert.equal(done.remaining.length, 0);
    assert.equal(done.messages.filter((m) => m.role === "user").length, 2);
  });

  it("one assistant body even if a turn_finished preview repeats it", () => {
    const preview = "我可以：\n\n- **读代码**\n- 修 bug";
    const messages = applyOptimisticUsers(parseTranscriptMessages(TAIL), []).messages;
    const assistants = messages.filter((m) => m.role === "assistant" && m.content === preview);
    assert.equal(assistants.length, 1);
    assert.equal(messages.some((m) => m.content.includes("hub:turn_finished")), false);
    assert.equal(messages.some((m) => m.content.includes("—— live events ——")), false);
  });

  it("clears an id-less user row once the unidentified count grows", () => {
    const tail: ChatMessage[] = [{ role: "user", content: "hi" }];
    const snap = snapshotUserContent(tail, "hi");
    assert.equal(snap.unidentifiedCount, 1);
    const pending: PendingUserSend[] = [
      {
        localId: "local-3",
        content: "hi",
        seenIds: snap.seenIds,
        unidentifiedCount: snap.unidentifiedCount,
      },
    ];
    const waiting = applyOptimisticUsers(tail, pending);
    assert.equal(waiting.remaining.length, 1);
    const grown: ChatMessage[] = [
      { role: "user", content: "hi" },
      { role: "assistant", content: "yo" },
      { role: "user", content: "hi" },
    ];
    const done = applyOptimisticUsers(grown, waiting.remaining);
    assert.equal(done.remaining.length, 0);
    assert.equal(done.messages.filter((m) => m.role === "user").length, 2);
  });
});
