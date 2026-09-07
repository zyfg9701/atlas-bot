import XCTest
@testable import AtlasBot

final class HubClientEncodingTests: XCTestCase {
    func testHelloFrameIsBotClient() throws {
        let h = RpcFrames.hello()
        XCTAssertEqual(h["protocol_version"] as? String, PROTOCOL_VERSION)
        XCTAssertEqual(h["kind"] as? String, "bot_client")
        XCTAssertNil(h["jsonrpc"])
    }

    func testRpcFrameJsonrpc2() throws {
        let f = RpcFrames.rpc(id: 7, method: "bot.status", params: [:])
        XCTAssertEqual(f["jsonrpc"] as? String, "2.0")
        XCTAssertEqual(f["id"] as? Int, 7)
        XCTAssertEqual(f["method"] as? String, "bot.status")
    }

    func testSendPromptArgsKeepAgentId() throws {
        let args = RpcFrames.sendPromptArgs(agentId: "agt_1", prompt: "hi", immediate: true)
        XCTAssertEqual(args["agentId"] as? String, "agt_1")
        XCTAssertEqual(args["prompt"] as? String, "hi")
        XCTAssertEqual(args["immediate"] as? Bool, true)
        let cmd = RpcFrames.command(agentId: "agt_1", name: "sendPrompt", args: args)
        XCTAssertEqual(cmd["agentId"] as? String, "agt_1")
        let nested = cmd["args"] as? [String: Any]
        XCTAssertEqual(nested?["agentId"] as? String, "agt_1")
    }

    func testTranscriptTailUsesId() throws {
        let args = RpcFrames.transcriptTailArgs(agentId: "agt_2", limit: 10)
        XCTAssertEqual(args["id"] as? String, "agt_2")
        XCTAssertEqual(args["limit"] as? Int, 10)
    }

    func testSubscribeCamelCase() throws {
        let p = RpcFrames.subscribe(agentIds: ["agt_1", "agt_2"])
        XCTAssertEqual((p["agentIds"] as? [String])?.count, 2)
        XCTAssertNil(p["fullFidelity"])
    }

    func testNormalizeErrorKeepsMismatch() throws {
        let err = normalizeError([
            "code": -32000,
            "message": "command_rejected",
            "data": ["reason": "agent_id_mismatch", "retryable": false],
        ] as [String: Any])
        XCTAssertEqual(err.message, "command_rejected")
        XCTAssertEqual(err.reason, "agent_id_mismatch")
        XCTAssertEqual(err.retryable, false)
    }

    func testNormalizeErrorUnknownCode() throws {
        let err = normalizeError(["message": "totally_new_code"] as [String: Any])
        XCTAssertEqual(err.code as? String, "upstream_error")
        XCTAssertTrue(err.message.contains("failure"))
    }

    func testNormalizeErrorGarbage() throws {
        let err = normalizeError(nil)
        XCTAssertEqual(err.code as? String, "upstream_error")
    }

    func testGeneratedEnvelopeConstant() throws {
        XCTAssertEqual(BOT_EVENT_ENVELOPE_V, 1)
    }
}
