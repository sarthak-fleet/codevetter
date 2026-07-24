import AppKit
import SwiftUI

private enum IslandPalette {
    static let background = Color(red: 0.035, green: 0.038, blue: 0.042)
    static let surface = Color(red: 0.07, green: 0.075, blue: 0.082)
    static var line: Color {
        Color.white.opacity(NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast ? 0.28 : 0.1)
    }
    static let primary = Color.white.opacity(0.94)
    static let secondary = Color.white.opacity(0.58)
    static let amber = Color(red: 0.83, green: 0.63, blue: 0.22)
    static let green = Color(red: 0.38, green: 0.83, blue: 0.65)
    static let red = Color(red: 0.93, green: 0.42, blue: 0.43)
}

struct IslandView: View {
    @ObservedObject var model: IslandModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Group {
            if model.expanded {
                expandedBody
            } else {
                collapsedBody
            }
        }
        .background(IslandPalette.background)
        .clipShape(RoundedRectangle(cornerRadius: model.expanded ? 22 : 16, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: model.expanded ? 22 : 16, style: .continuous)
                .stroke(IslandPalette.line, lineWidth: 1)
        )
        .shadow(color: Color.black.opacity(0.45), radius: 24, x: 0, y: 10)
        .transaction { transaction in
            if reduceMotion {
                transaction.animation = nil
            }
        }
        .onExitCommand {
            if model.expanded {
                model.toggleExpanded()
            }
        }
    }

    private var collapsedBody: some View {
        Button(action: model.toggleExpanded) {
            HStack(spacing: 10) {
                statusDot(model.primarySession?.status)
                VStack(alignment: .leading, spacing: 1) {
                    Text(collapsedTitle)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundColor(IslandPalette.primary)
                        .lineLimit(1)
                    Text(collapsedSubtitle)
                        .font(.system(size: 11, weight: .regular))
                        .foregroundColor(IslandPalette.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 8)
                if model.sessions.count > 1 {
                    Text("\(model.sessions.count)")
                        .font(.system(size: 10, weight: .semibold, design: .rounded))
                        .foregroundColor(IslandPalette.secondary)
                        .padding(.horizontal, 7)
                        .padding(.vertical, 4)
                        .background(IslandPalette.surface)
                        .clipShape(Capsule())
                }
                Image(systemName: "chevron.down")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundColor(IslandPalette.secondary)
            }
            .padding(.horizontal, 15)
            .frame(width: 320, height: 48)
            .contentShape(Rectangle())
        }
        .buttonStyle(PlainButtonStyle())
        .accessibilityLabel(collapsedAccessibilityLabel)
    }

    private var expandedBody: some View {
        VStack(spacing: 0) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Agents")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundColor(IslandPalette.primary)
                    Text(model.latestOutcome ?? summaryText)
                        .font(.system(size: 11))
                        .foregroundColor(
                            model.latestOutcome == nil ? IslandPalette.secondary : IslandPalette.amber
                        )
                }
                Spacer()
                Button(action: model.toggleExpanded) {
                    Image(systemName: "chevron.up")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundColor(IslandPalette.secondary)
                        .frame(width: 28, height: 28)
                        .background(IslandPalette.surface)
                        .clipShape(Circle())
                }
                .buttonStyle(PlainButtonStyle())
                .accessibilityLabel("Collapse agent island")
            }
            .padding(.horizontal, 18)
            .padding(.vertical, 15)

            Rectangle()
                .fill(IslandPalette.line)
                .frame(height: 1)

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    ForEach(model.groupedSessions, id: \.0) { project, sessions in
                        VStack(alignment: .leading, spacing: 7) {
                            Text(project.uppercased())
                                .font(.system(size: 9, weight: .semibold))
                                .tracking(1.2)
                                .foregroundColor(IslandPalette.secondary)
                                .padding(.horizontal, 3)
                            ForEach(sessions) { session in
                                sessionRow(session)
                            }
                        }
                    }
                }
                .padding(12)
            }
            .frame(maxHeight: 390)
        }
        .frame(width: 420)
    }

    private func sessionRow(_ session: AgentSession) -> some View {
        AgentSessionRow(model: model, session: session)
    }

    private var collapsedTitle: String {
        guard let session = model.primarySession else { return "CodeVetter" }
        return "\(providerName(session.provider)) · \(session.status.label)"
    }

    private var collapsedSubtitle: String {
        guard let session = model.primarySession else { return "No active agents" }
        return "\(session.project) · \(session.reason)"
    }

    private var collapsedAccessibilityLabel: String {
        guard let session = model.primarySession else { return "CodeVetter, no active agents" }
        return accessibilitySnapshot(for: session, expanded: false).summary
    }

    private var summaryText: String {
        let help = model.sessions.filter { $0.status == .needsHelp }.count
        let working = model.sessions.filter { $0.status == .working }.count
        if help > 0 { return "\(help) need\(help == 1 ? "s" : "") you · \(working) working" }
        if working > 0 { return "\(working) working across \(model.groupedSessions.count) projects" }
        return "\(model.sessions.count) recent sessions"
    }

    private func providerName(_ provider: String) -> String {
        provider == "claude" ? "Claude" : "Codex"
    }

    private func providerMark(_ provider: String) -> some View {
        ZStack {
            Circle()
                .fill(Color.white.opacity(0.055))
                .frame(width: 30, height: 30)
            Image(systemName: provider == "claude" ? "sparkles" : "chevron.left.forwardslash.chevron.right")
                .font(.system(size: 11, weight: .semibold))
                .foregroundColor(provider == "claude" ? IslandPalette.amber : IslandPalette.green)
        }
        .accessibilityHidden(true)
    }

    private func statusDot(_ status: AgentStatus?) -> some View {
        Circle()
            .fill(statusColor(status ?? .paused))
            .frame(width: 8, height: 8)
            .shadow(color: statusColor(status ?? .paused).opacity(0.35), radius: 5)
            .accessibilityHidden(true)
    }

    private func statusColor(_ status: AgentStatus) -> Color {
        switch status {
        case .needsHelp: return IslandPalette.amber
        case .failed: return IslandPalette.red
        case .completed: return IslandPalette.green
        case .working: return IslandPalette.green
        case .paused, .disconnected: return IslandPalette.secondary
        }
    }
}

private struct AgentSessionRow: View {
    @ObservedObject var model: IslandModel
    let session: AgentSession

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 11) {
                providerMark
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 7) {
                        Text(providerName)
                            .font(.system(size: 12, weight: .semibold))
                            .foregroundColor(IslandPalette.primary)
                        Text(session.status.label)
                            .font(.system(size: 10, weight: .medium))
                            .foregroundColor(statusColor)
                        Text(ageLabel)
                            .font(.system(size: 10))
                            .foregroundColor(IslandPalette.secondary)
                    }
                    Text(session.reason)
                        .font(.system(size: 11))
                        .foregroundColor(IslandPalette.secondary)
                        .lineLimit(2)
                }
                Spacer(minLength: 6)
                if session.capabilities.canFocus {
                    Button("Open") {
                        model.send(action: "focus_session", for: session)
                    }
                    .buttonStyle(IslandActionButtonStyle())
                    .accessibilityLabel("Open \(providerName) in \(session.project)")
                }
                if session.capabilities.canDismiss {
                    Button(action: { model.send(action: "dismiss", for: session) }) {
                        Image(systemName: "xmark")
                            .font(.system(size: 9, weight: .semibold))
                    }
                    .buttonStyle(IslandIconButtonStyle())
                    .accessibilityLabel("Dismiss \(providerName) status")
                }
            }

            if session.capabilities.canApprove || session.capabilities.canDeny {
                HStack(spacing: 8) {
                if session.capabilities.canDeny {
                    Button("Deny") {
                        model.send(action: "deny", for: session)
                    }
                    .buttonStyle(IslandActionButtonStyle())
                    .accessibilityLabel("Deny \(providerName) request")
                }
                if session.capabilities.canApprove {
                    Button("Approve once") {
                        model.send(action: "approve", for: session)
                    }
                    .buttonStyle(IslandPrimaryButtonStyle())
                    .accessibilityLabel("Approve \(providerName) request once")
                }
                }
                .padding(.leading, 41)
            }

            if session.capabilities.canReply {
                HStack(spacing: 8) {
                    TextField("Reply…", text: replyBinding, onCommit: sendReply)
                        .textFieldStyle(PlainTextFieldStyle())
                        .font(.system(size: 11))
                        .foregroundColor(IslandPalette.primary)
                        .padding(.horizontal, 10)
                        .frame(height: 30)
                        .background(Color.black.opacity(0.28))
                        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                        .accessibilityLabel("Reply to \(providerName)")
                    Button("Send", action: sendReply)
                        .buttonStyle(IslandPrimaryButtonStyle())
                        .disabled(replyValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        .accessibilityLabel("Send reply to \(providerName)")
                }
                .padding(.leading, 41)
            }
        }
        .padding(12)
        .background(IslandPalette.surface)
        .clipShape(RoundedRectangle(cornerRadius: 13, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 13, style: .continuous)
                .stroke(statusColor.opacity(0.22), lineWidth: 1)
        )
        .accessibilityElement(children: .contain)
        .accessibilityLabel(accessibilitySnapshot(for: session, expanded: true).summary)
    }

    private func sendReply() {
        let value = replyValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else { return }
        model.send(action: "submit_reply", for: session, value: value)
        model.clearReplyDraft(for: session.sessionID)
    }

    private var replyValue: String {
        model.replyDraft(for: session.sessionID)
    }

    private var replyBinding: Binding<String> {
        Binding(
            get: { model.replyDraft(for: session.sessionID) },
            set: { model.setReplyDraft($0, for: session.sessionID) }
        )
    }

    private var providerName: String {
        session.provider == "claude" ? "Claude" : "Codex"
    }

    private var ageLabel: String {
        let now = UInt64(Date().timeIntervalSince1970 * 1_000)
        let seconds = now > session.updatedAtMilliseconds
            ? (now - session.updatedAtMilliseconds) / 1_000
            : 0
        if seconds < 60 { return "\(seconds)s" }
        if seconds < 3_600 { return "\(seconds / 60)m" }
        return "\(seconds / 3_600)h"
    }

    private var providerMark: some View {
        ZStack {
            Circle()
                .fill(Color.white.opacity(0.055))
                .frame(width: 30, height: 30)
            Image(
                systemName: session.provider == "claude"
                    ? "sparkles"
                    : "chevron.left.forwardslash.chevron.right"
            )
            .font(.system(size: 11, weight: .semibold))
            .foregroundColor(session.provider == "claude" ? IslandPalette.amber : IslandPalette.green)
        }
        .accessibilityHidden(true)
    }

    private var statusColor: Color {
        switch session.status {
        case .needsHelp: return IslandPalette.amber
        case .failed: return IslandPalette.red
        case .completed, .working: return IslandPalette.green
        case .paused, .disconnected: return IslandPalette.secondary
        }
    }
}

private struct IslandActionButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 10, weight: .semibold))
            .foregroundColor(IslandPalette.primary)
            .padding(.horizontal, 10)
            .frame(height: 28)
            .background(configuration.isPressed ? Color.white.opacity(0.13) : Color.white.opacity(0.08))
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

private struct IslandPrimaryButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 10, weight: .semibold))
            .foregroundColor(Color.black.opacity(0.88))
            .padding(.horizontal, 11)
            .frame(height: 28)
            .background(configuration.isPressed ? IslandPalette.amber.opacity(0.75) : IslandPalette.amber)
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

private struct IslandIconButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundColor(IslandPalette.secondary)
            .frame(width: 28, height: 28)
            .background(configuration.isPressed ? Color.white.opacity(0.1) : Color.clear)
            .clipShape(Circle())
    }
}
