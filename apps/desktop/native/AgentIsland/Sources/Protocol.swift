import Foundation

let protocolVersion: UInt16 = 1
let maximumMessageBytes = 64 * 1024

enum AgentStatus: String, Codable, CaseIterable {
    case working
    case needsHelp = "needs_help"
    case failed
    case completed
    case paused
    case disconnected

    var priority: Int {
        switch self {
        case .needsHelp: return 0
        case .failed: return 1
        case .completed: return 2
        case .working: return 3
        case .paused: return 4
        case .disconnected: return 5
        }
    }

    var label: String {
        switch self {
        case .needsHelp: return "Needs help"
        case .failed: return "Failed"
        case .completed: return "Completed"
        case .working: return "Working"
        case .paused: return "Paused"
        case .disconnected: return "Disconnected"
        }
    }
}

struct AgentCapabilities: Codable, Equatable {
    let canFocus: Bool
    let canReply: Bool
    let canApprove: Bool
    let canDeny: Bool
    let canSnooze: Bool
    let canDismiss: Bool

    enum CodingKeys: String, CodingKey {
        case canFocus = "can_focus"
        case canReply = "can_reply"
        case canApprove = "can_approve"
        case canDeny = "can_deny"
        case canSnooze = "can_snooze"
        case canDismiss = "can_dismiss"
    }
}

struct AgentSession: Codable, Identifiable, Equatable {
    let sessionID: String
    let eventID: String
    let provider: String
    let project: String
    let status: AgentStatus
    let reason: String
    let confirmed: Bool
    let startedAtMilliseconds: UInt64
    let updatedAtMilliseconds: UInt64
    let capabilities: AgentCapabilities

    var id: String { sessionID }

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case eventID = "event_id"
        case provider
        case project
        case status
        case reason
        case confirmed
        case startedAtMilliseconds = "started_at_ms"
        case updatedAtMilliseconds = "updated_at_ms"
        case capabilities
    }
}

struct IslandAccessibilitySnapshot: Equatable {
    let summary: String
    let actions: [String]
}

func accessibilitySnapshot(for session: AgentSession, expanded: Bool) -> IslandAccessibilitySnapshot {
    let provider = session.provider == "claude" ? "Claude" : "Codex"
    let summary = "\(provider), \(session.project), \(session.status.label), \(session.reason)"
    guard expanded else {
        return IslandAccessibilitySnapshot(
            summary: "\(summary). Expand agent island.",
            actions: ["Expand agent island"]
        )
    }

    var actions = [String]()
    if session.capabilities.canFocus {
        actions.append("Open \(provider) in \(session.project)")
    }
    if session.capabilities.canDeny {
        actions.append("Deny \(provider) request")
    }
    if session.capabilities.canApprove {
        actions.append("Approve \(provider) request once")
    }
    if session.capabilities.canReply {
        actions.append("Reply to \(provider)")
        actions.append("Send reply to \(provider)")
    }
    if session.capabilities.canDismiss {
        actions.append("Dismiss \(provider) status")
    }
    return IslandAccessibilitySnapshot(summary: summary, actions: actions)
}

struct SpeechSettings: Codable, Equatable {
    let muted: Bool
    let completionEnabled: Bool
    let attentionEnabled: Bool
    let failureEnabled: Bool
    let codexVoice: String?
    let claudeVoice: String?
    let rate: Float
    let volume: Float
    let quietHoursStart: UInt8?
    let quietHoursEnd: UInt8?
    let cooldownSeconds: UInt64

    enum CodingKeys: String, CodingKey {
        case muted
        case completionEnabled = "completion_enabled"
        case attentionEnabled = "attention_enabled"
        case failureEnabled = "failure_enabled"
        case codexVoice = "codex_voice"
        case claudeVoice = "claude_voice"
        case rate
        case volume
        case quietHoursStart = "quiet_hours_start"
        case quietHoursEnd = "quiet_hours_end"
        case cooldownSeconds = "cooldown_seconds"
    }
}

struct IslandSettings: Codable, Equatable {
    let enabled: Bool
    let speech: SpeechSettings
}

struct IslandSnapshot: Codable, Equatable {
    let sessions: [AgentSession]
    let settings: IslandSettings
    let preview: Bool
}

struct ActionResult: Codable, Equatable {
    let requestSequence: UInt64
    let sessionID: String?
    let eventID: String?
    let disposition: String
    let error: String?

    enum CodingKeys: String, CodingKey {
        case requestSequence = "request_seq"
        case sessionID = "session_id"
        case eventID = "event_id"
        case disposition
        case error
    }
}

struct IncomingEnvelope: Decodable {
    let version: UInt16
    let sequence: UInt64
    let sentAtMilliseconds: UInt64
    let kind: String
    let payload: IslandSnapshot?
    let actionResult: ActionResult?

    enum CodingKeys: String, CodingKey {
        case version = "v"
        case sequence = "seq"
        case sentAtMilliseconds = "sent_at_ms"
        case kind
        case payload
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        version = try values.decode(UInt16.self, forKey: .version)
        sequence = try values.decode(UInt64.self, forKey: .sequence)
        sentAtMilliseconds = try values.decode(UInt64.self, forKey: .sentAtMilliseconds)
        kind = try values.decode(String.self, forKey: .kind)
        if kind == "snapshot" {
            payload = try values.decode(IslandSnapshot.self, forKey: .payload)
            actionResult = nil
        } else if kind == "action_result" {
            payload = nil
            actionResult = try values.decode(ActionResult.self, forKey: .payload)
        } else {
            payload = nil
            actionResult = nil
        }
    }
}

struct OutgoingEnvelope<Payload: Encodable>: Encodable {
    let version: UInt16
    let sequence: UInt64
    let sentAtMilliseconds: UInt64
    let kind: String
    let payload: Payload

    enum CodingKeys: String, CodingKey {
        case version = "v"
        case sequence = "seq"
        case sentAtMilliseconds = "sent_at_ms"
        case kind
        case payload
    }
}

struct AgentIntent: Encodable {
    let action: String
    let sessionID: String
    let eventID: String
    let value: String?

    enum CodingKeys: String, CodingKey {
        case action
        case sessionID = "session_id"
        case eventID = "event_id"
        case value
    }
}

struct RenderAcknowledgement: Encodable {
    let sourceSequence: UInt64
    let receivedAtMilliseconds: UInt64
    let appliedAtMilliseconds: UInt64

    enum CodingKeys: String, CodingKey {
        case sourceSequence = "source_seq"
        case receivedAtMilliseconds = "received_at_ms"
        case appliedAtMilliseconds = "applied_at_ms"
    }
}

enum ProtocolParser {
    static func decode(_ data: Data) throws -> IncomingEnvelope {
        guard !data.isEmpty, data.count <= maximumMessageBytes else {
            throw ProtocolError.invalidSize
        }
        let envelope = try JSONDecoder().decode(IncomingEnvelope.self, from: data)
        guard envelope.version == protocolVersion,
              envelope.sequence > 0,
              envelope.sentAtMilliseconds > 0
        else {
            throw ProtocolError.unsupportedEnvelope
        }
        return envelope
    }
}

enum ProtocolError: Error {
    case invalidSize
    case unsupportedEnvelope
}
