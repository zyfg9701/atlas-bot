import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { normalizeError, sortEventsBySeq, type BotEventEnvelope } from "./hubClient";

describe("normalizeError", () => {
  it("keeps known command_rejected", () => {
    const e = normalizeError({
      code: -32000,
      message: "command_rejected",
      data: { reason: "agent_id_mismatch", retryable: false },
    });
    assert.equal(e.message, "command_rejected");
    assert.equal(e.reason, "agent_id_mismatch");
    assert.equal(e.retryable, false);
  });

  it("maps unknown wire code to failure/upstream_error", () => {
    const e = normalizeError({
      message: "totally_new_code",
      data: { retryable: false },
    });
    assert.equal(e.code, "upstream_error");
    assert.match(e.message, /failure/);
  });

  it("does not throw on garbage", () => {
    const e = normalizeError(null);
    assert.equal(e.code, "upstream_error");
  });
});

describe("sortEventsBySeq", () => {
  it("sorts by agent then seq without dedupe", () => {
    const evs: BotEventEnvelope[] = [
      { v: 1, agentId: "agt_1", seq: 3, channel: "hub:turn_finished", event: {} },
      { v: 1, agentId: "agt_2", seq: 1, channel: "hub:turn_finished", event: {} },
      { v: 1, agentId: "agt_1", seq: 1, channel: "hub:turn_finished", event: {} },
      { v: 1, agentId: "agt_1", seq: 2, channel: "hub:turn_finished", event: {} },
    ];
    const s = sortEventsBySeq(evs);
    assert.deepEqual(
      s.map((e) => `${e.agentId}:${e.seq}`),
      ["agt_1:1", "agt_1:2", "agt_1:3", "agt_2:1"],
    );
  });
});
