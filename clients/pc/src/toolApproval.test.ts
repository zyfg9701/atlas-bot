import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  DEFAULT_GW_HTTP,
  approvalCardVisible,
  approvalFetchInit,
  approvalQueueView,
  approvalSettled,
  decisionLabel,
  forgetPending,
  formatApprovalFailure,
  ingestHubToolPending,
  parseApprovalPending,
  rememberPending,
  resolveGatewayHttpBase,
  type PendingApproval,
} from "./toolApproval";

const SHELL = "[approval_pending approvalId=appr_shell tool=Shell] mkdir out";
const WRITE = "[approval_pending approvalId=appr_write tool=Write] notes.txt";
const RUN = "[approval_pending approvalId=appr_run tool=RUN mkdir] workspace/tmp";

function row(partial: Partial<PendingApproval> & Pick<PendingApproval, "approvalId" | "agentId">): PendingApproval {
  return {
    tool: "Shell",
    summary: SHELL,
    ...partial,
  };
}

describe("parseApprovalPending", () => {
  it("parses GA1b / CG1 prefix including spaced tool names", () => {
    assert.deepEqual(parseApprovalPending(SHELL), { approvalId: "appr_shell", tool: "Shell" });
    assert.deepEqual(parseApprovalPending(`  ${RUN}  `), { approvalId: "appr_run", tool: "RUN mkdir" });
  });

  it("ignores post-exec hub:tool and empty text", () => {
    assert.equal(parseApprovalPending("tool=Shell exit=0"), null);
    assert.equal(parseApprovalPending("[approval_pending] missing fields"), null);
    assert.equal(parseApprovalPending(""), null);
  });
});

describe("decision labels", () => {
  it("uses Chinese Chat labels", () => {
    assert.equal(decisionLabel("allow"), "允许");
    assert.equal(decisionLabel("deny"), "拒绝");
  });
});

describe("queue", () => {
  it("keeps every pending id when a newer one arrives", () => {
    const q = new Map<string, PendingApproval>();
    rememberPending(q, row({ approvalId: "appr_shell", agentId: "agt_1", tool: "Shell", summary: SHELL }));
    rememberPending(q, row({ approvalId: "appr_write", agentId: "agt_1", tool: "Write", summary: WRITE }));
    rememberPending(q, row({ approvalId: "appr_run", agentId: "agt_2", tool: "RUN mkdir", summary: RUN }));
    assert.deepEqual([...q.keys()], ["appr_shell", "appr_write", "appr_run"]);

    const view = approvalQueueView(q, "agt_1");
    assert.deepEqual(
      view.mine.map((p) => p.approvalId),
      ["appr_shell", "appr_write"],
    );
    assert.equal(view.otherCount, 1);
    assert.equal(approvalCardVisible(view), true);
    assert.equal(approvalCardVisible(approvalQueueView(q, "agt_2")), true);
    assert.equal(approvalCardVisible(approvalQueueView(new Map(), "agt_1")), false);
  });

  it("refreshes the same id without dropping siblings or reordering", () => {
    const q = new Map<string, PendingApproval>();
    rememberPending(q, row({ approvalId: "a", agentId: "agt_1" }));
    rememberPending(q, row({ approvalId: "b", agentId: "agt_1", tool: "Write" }));
    rememberPending(q, row({ approvalId: "a", agentId: "agt_1", tool: "Bash", summary: "updated" }));
    assert.deepEqual([...q.keys()], ["a", "b"]);
    assert.equal(q.get("a")?.tool, "Bash");
    assert.equal(q.get("a")?.summary, "updated");
    assert.equal(forgetPending(q, "a"), true);
    assert.deepEqual([...q.keys()], ["b"]);
    assert.equal(forgetPending(q, "missing"), false);
  });

  it("ingests hub:tool pending for any agent and ignores other channels", () => {
    const q = new Map<string, PendingApproval>();
    const stored = ingestHubToolPending(q, {
      channel: "hub:tool",
      agentId: "agt_9",
      event: { summary: WRITE, tool: "approval_pending", exitCode: null },
    });
    assert.equal(stored?.approvalId, "appr_write");
    assert.equal(q.get("appr_write")?.agentId, "agt_9");
    assert.equal(
      ingestHubToolPending(q, {
        channel: "hub:turn_finished",
        agentId: "agt_9",
        event: { summary: SHELL },
      }),
      null,
    );
    assert.equal(
      ingestHubToolPending(q, {
        channel: "hub:tool",
        agentId: "agt_9",
        event: { summary: "completed Shell", exitCode: 0 },
      }),
      null,
    );
    assert.equal(q.size, 1);
  });
});

describe("gateway HTTP", () => {
  it("defaults to loopback and strips a trailing slash", () => {
    assert.equal(resolveGatewayHttpBase(null), DEFAULT_GW_HTTP);
    assert.equal(resolveGatewayHttpBase("  "), DEFAULT_GW_HTTP);
    assert.equal(resolveGatewayHttpBase("http://127.0.0.1:8787/"), DEFAULT_GW_HTTP);
    assert.equal(resolveGatewayHttpBase("http://127.0.0.1:9797"), "http://127.0.0.1:9797");
  });

  it("posts approvalId and decision, attaching the token only when set", () => {
    const bare = approvalFetchInit("appr_shell", "deny", "  ");
    assert.deepEqual(JSON.parse(bare.body), { approvalId: "appr_shell", decision: "deny" });
    assert.equal(bare.headers.Authorization, undefined);
    assert.equal(bare.headers["X-Atlas-Approval-Token"], undefined);
    assert.equal(bare.headers["Content-Type"], "application/json");

    const authed = approvalFetchInit("appr_shell", "allow", "sekret");
    assert.equal(authed.headers.Authorization, "Bearer sekret");
    assert.equal(authed.headers["X-Atlas-Approval-Token"], "sekret");
    assert.deepEqual(JSON.parse(authed.body), { approvalId: "appr_shell", decision: "allow" });
  });

  it("treats 2xx and 409 as settled and keeps other failures", () => {
    assert.equal(approvalSettled(200), true);
    assert.equal(approvalSettled(204), true);
    assert.equal(approvalSettled(409), true);
    assert.equal(approvalSettled(401), false);
    assert.equal(approvalSettled(500), false);
    assert.equal(approvalSettled(0), false);
  });

  it("formats Chat-visible Chinese failures", () => {
    assert.match(formatApprovalFailure({ decision: "allow", networkError: "Failed to fetch" }), /允许/);
    assert.match(formatApprovalFailure({ decision: "allow", networkError: "Failed to fetch" }), /无法连接 Gateway/);
    const http = formatApprovalFailure({ decision: "deny", status: 401, body: "unauthorized\n" });
    assert.match(http, /拒绝/);
    assert.match(http, /HTTP 401/);
    assert.match(http, /unauthorized/);
    assert.doesNotMatch(http, /\n/);
  });
});
