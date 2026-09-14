import Foundation
import Testing
@testable import TraverseEmbedder

@Test func hostConnectorCommandContractIsSharedWithBrowser() throws {
    let command = try HostConnectorAppCommand.audioCapture(
        commandId: "cmd-1",
        correlationId: "corr-1",
        idempotencyKey: "idem-1",
        targetFamily: "macos",
        maxDurationMs: 5000,
        maxBytes: 1_048_576
    )
    let wire = try command.wireObject()
    #expect(wire["kind"] as? String == HostConnectorCommandContract.commandKind)
    #expect(wire["schema_version"] as? String == HostConnectorCommandContract.schemaVersion)
    #expect(wire["command"] as? String == "capture_audio")
    #expect(wire["target_family"] as? String == "macos")
    #expect(HostConnectorCommandContract.audioCaptureOperation == "audio.capture")
    #expect(HostConnectorCommandContract.eventKind == "host_connector_event")

    let browser = try HostConnectorAppCommand.audioCapture(
        commandId: "cmd-1",
        correlationId: "corr-1",
        idempotencyKey: "idem-1",
        targetFamily: "browser",
        maxDurationMs: 5000,
        maxBytes: 1_048_576
    )
    let browserWire = try browser.wireObject()
    #expect(Set(wire.keys) == Set(browserWire.keys))

    let event = HostConnectorEvent(
        event: "failed",
        commandId: "cmd-1",
        correlationId: "corr-1",
        targetFamily: "browser",
        errorCode: "target_incompatible"
    )
    let eventWire = event.wireObject()
    #expect(eventWire["kind"] as? String == "host_connector_event")
    #expect(eventWire["error_code"] as? String == "target_incompatible")
    let encoded = String(describing: wire)
    #expect(!encoded.contains("microphone"))
}
