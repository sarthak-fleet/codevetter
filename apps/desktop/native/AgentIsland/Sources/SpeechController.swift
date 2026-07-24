import AVFoundation
import Foundation

final class SpeechController {
    private let synthesizer = AVSpeechSynthesizer()
    private var spokenEvents: [String: Date] = [:]

    func apply(previous: [AgentSession], snapshot: IslandSnapshot) {
        guard !snapshot.preview, !snapshot.settings.speech.muted else { return }
        let previousByID = Dictionary(uniqueKeysWithValues: previous.map { ($0.sessionID, $0) })

        for session in snapshot.sessions {
            guard previousByID[session.sessionID]?.eventID != session.eventID,
                  shouldSpeak(session.status, settings: snapshot.settings.speech),
                  !isQuietHour(
                    start: snapshot.settings.speech.quietHoursStart,
                    end: snapshot.settings.speech.quietHoursEnd,
                    hour: Calendar.current.component(.hour, from: Date())
                  ),
                  isOutsideCooldown(session, settings: snapshot.settings.speech)
            else {
                continue
            }
            speak(session, settings: snapshot.settings.speech)
        }
    }

    private func shouldSpeak(_ status: AgentStatus, settings: SpeechSettings) -> Bool {
        switch status {
        case .completed: return settings.completionEnabled
        case .needsHelp: return settings.attentionEnabled
        case .failed: return settings.failureEnabled
        case .working, .paused, .disconnected: return false
        }
    }

    private func isOutsideCooldown(_ session: AgentSession, settings: SpeechSettings) -> Bool {
        let now = Date()
        let key = "\(session.sessionID):\(session.status.rawValue)"
        if let previous = spokenEvents[key],
           now.timeIntervalSince(previous) < TimeInterval(settings.cooldownSeconds)
        {
            return false
        }
        spokenEvents[key] = now
        if spokenEvents.count > 128 {
            spokenEvents = spokenEvents.filter {
                now.timeIntervalSince($0.value) < TimeInterval(settings.cooldownSeconds * 4)
            }
        }
        return true
    }

    private func speak(_ session: AgentSession, settings: SpeechSettings) {
        let provider = session.provider == "claude" ? "Claude" : "Codex"
        let phrase: String
        switch session.status {
        case .needsHelp:
            phrase = "\(provider) needs you in \(session.project)"
        case .failed:
            phrase = "\(provider) hit a problem in \(session.project)"
        case .completed:
            phrase = "\(provider) finished in \(session.project)"
        case .working, .paused, .disconnected:
            return
        }

        if session.status == .needsHelp, synthesizer.isSpeaking {
            synthesizer.stopSpeaking(at: .immediate)
        }
        let utterance = AVSpeechUtterance(string: phrase)
        utterance.rate = min(max(settings.rate, 0.0), 1.0)
        utterance.volume = min(max(settings.volume, 0.0), 1.0)

        let configured = session.provider == "claude"
            ? settings.claudeVoice
            : settings.codexVoice
        if let configured,
           let voice = AVSpeechSynthesisVoice(identifier: configured)
        {
            utterance.voice = voice
        } else {
            let language = session.provider == "claude" ? "en-GB" : "en-US"
            utterance.voice = AVSpeechSynthesisVoice(language: language)
        }
        synthesizer.speak(utterance)
    }
}

func isQuietHour(start: UInt8?, end: UInt8?, hour: Int) -> Bool {
    guard let start, let end else { return false }
    if start == end { return true }
    if start < end {
        return hour >= Int(start) && hour < Int(end)
    }
    return hour >= Int(start) || hour < Int(end)
}
