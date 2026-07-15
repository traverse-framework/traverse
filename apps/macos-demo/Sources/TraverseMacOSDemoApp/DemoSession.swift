import Foundation

struct DemoSession: Decodable {
    let demoID: String
    let title: String
    let requestID: String
    let executionID: String
    let traceID: String
    let status: String
    let summary: String
    let request: DemoRequest
    let stateUpdates: [DemoStateUpdate]
    let trace: DemoTrace

    enum CodingKeys: String, CodingKey {
        case demoID = "demo_id"
        case title
        case requestID = "request_id"
        case executionID = "execution_id"
        case traceID = "trace_id"
        case status
        case summary
        case request
        case stateUpdates = "state_updates"
        case trace
    }
}

struct DemoRequest: Decodable {
    let goal: String
    let requestedTarget: String
    let caller: String

    enum CodingKeys: String, CodingKey {
        case goal
        case requestedTarget = "requested_target"
        case caller
    }
}

struct DemoStateUpdate: Decodable, Identifiable {
    let state: String
    let title: String
    let timestamp: String
    let detail: String

    var id: String { "\(timestamp)-\(state)" }
}

struct DemoTrace: Decodable {
    let selectedCapabilityID: String
    let selectedCapabilityVersion: String
    let placement: DemoPlacement
    let emittedEvents: [String]
    let output: DemoOutput

    enum CodingKeys: String, CodingKey {
        case selectedCapabilityID = "selected_capability_id"
        case selectedCapabilityVersion = "selected_capability_version"
        case placement
        case emittedEvents = "emitted_events"
        case output
    }
}

struct DemoPlacement: Decodable {
    let requestedTarget: String
    let selectedTarget: String
    let status: String
    let reason: String

    enum CodingKeys: String, CodingKey {
        case requestedTarget = "requested_target"
        case selectedTarget = "selected_target"
        case status
        case reason
    }
}

struct DemoOutput: Decodable {
    let planID: String
    let route: String
    let weatherSummary: String
    let teamStatus: String
    let nextAction: String

    enum CodingKeys: String, CodingKey {
        case planID = "plan_id"
        case route
        case weatherSummary = "weather_summary"
        case teamStatus = "team_status"
        case nextAction = "next_action"
    }
}

enum DemoSessionRepository {
    static func sample() -> DemoSession {
        let url = URL(fileURLWithPath: "examples/fixtures/expedition-runtime-session.json")
        let data = (try? Data(contentsOf: url)) ?? Data()
        let decoder = JSONDecoder()

        if let session = try? decoder.decode(DemoSession.self, from: data) {
            return session
        }

        return DemoSession(
            demoID: "fallback",
            title: "Traverse macOS Demo",
            requestID: "req-fallback",
            executionID: "exec-fallback",
            traceID: "trace-fallback",
            status: "error",
            summary: "Fixture loading failed.",
            request: DemoRequest(goal: "Unavailable", requestedTarget: "local", caller: "native_demo"),
            stateUpdates: [],
            trace: DemoTrace(
                selectedCapabilityID: "unavailable",
                selectedCapabilityVersion: "0.0.0",
                placement: DemoPlacement(
                    requestedTarget: "local",
                    selectedTarget: "local",
                    status: "not_attempted",
                    reason: "fixture_unavailable"
                ),
                emittedEvents: [],
                output: DemoOutput(
                    planID: "unavailable",
                    route: "Unavailable",
                    weatherSummary: "Unavailable",
                    teamStatus: "unavailable",
                    nextAction: "Restore the fixture file."
                )
            )
        )
    }
}
