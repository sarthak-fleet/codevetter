import AppKit
import Combine
import Darwin
import Foundation
import SwiftUI

if CommandLine.arguments.contains("--self-test") {
    exit(runSelfTests())
}

final class IslandPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
}

final class IslandApplicationController: NSObject, NSApplicationDelegate {
    private let model = IslandModel()
    private var panel: IslandPanel?
    private var cancellables = Set<AnyCancellable>()
    private let parentPID: pid_t

    init(parentPID: pid_t) {
        self.parentPID = parentPID
        super.init()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        createPanel()
        startInputReader()
        startParentMonitor()

        NotificationCenter.default.addObserver(
            self,
            selector: #selector(reposition),
            name: NSApplication.didChangeScreenParametersNotification,
            object: nil
        )
    }

    private func createPanel() {
        let initialFrame = NSRect(x: 0, y: 0, width: 320, height: 48)
        let panel = IslandPanel(
            contentRect: initialFrame,
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        panel.isFloatingPanel = true
        panel.level = .statusBar
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = false
        panel.hidesOnDeactivate = false
        panel.becomesKeyOnlyIfNeeded = true
        panel.isReleasedWhenClosed = false
        panel.contentView = NSHostingView(rootView: IslandView(model: model))
        self.panel = panel

        model.$sessions
            .combineLatest(model.$expanded)
            .receive(on: RunLoop.main)
            .sink { [weak self] sessions, expanded in
                self?.updatePanel(sessions: sessions, expanded: expanded)
            }
            .store(in: &cancellables)

        model.$expanded
            .removeDuplicates()
            .dropFirst()
            .receive(on: RunLoop.main)
            .sink { [weak self] expanded in
                guard expanded, let panel = self?.panel else { return }
                panel.makeKeyAndOrderFront(nil)
                panel.recalculateKeyViewLoop()
                panel.selectNextKeyView(nil)
            }
            .store(in: &cancellables)
    }

    private func updatePanel(sessions: [AgentSession], expanded: Bool) {
        guard let panel else { return }
        guard !sessions.isEmpty else {
            panel.orderOut(nil)
            return
        }
        let visibleSessions = Array(sessions.prefix(6))
        let baseRowsHeight = visibleSessions.count * 74
        let actionRowsHeight = visibleSessions.reduce(0) { height, session in
            height
                + ((session.capabilities.canApprove || session.capabilities.canDeny) ? 38 : 0)
                + (session.capabilities.canReply ? 40 : 0)
        }
        let target = expanded
            ? NSSize(width: 420, height: min(540, 86 + baseRowsHeight + actionRowsHeight))
            : NSSize(width: 320, height: 48)
        panel.setContentSize(target)
        reposition()
        panel.orderFrontRegardless()
    }

    @objc private func reposition() {
        guard let panel, let screen = activeScreen() else { return }
        let safeAreaTop: CGFloat
        if #available(macOS 12.0, *) {
            safeAreaTop = screen.safeAreaInsets.top
        } else {
            safeAreaTop = 0
        }
        let safeTop = islandTopBoundary(
            frameMaxY: screen.frame.maxY,
            visibleFrameMaxY: screen.visibleFrame.maxY,
            safeAreaTop: safeAreaTop
        )
        let x = screen.frame.midX - panel.frame.width / 2
        let y = safeTop - panel.frame.height - 8
        panel.setFrameOrigin(NSPoint(x: x, y: y))
    }

    private func activeScreen() -> NSScreen? {
        let mouse = NSEvent.mouseLocation
        return NSScreen.screens.first(where: { NSMouseInRect(mouse, $0.frame, false) })
            ?? NSScreen.main
            ?? NSScreen.screens.first
    }

    private func startInputReader() {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            while let line = readLine(strippingNewline: true) {
                let receivedAtMilliseconds = UInt64(Date().timeIntervalSince1970 * 1_000)
                guard let data = line.data(using: .utf8),
                      let envelope = try? ProtocolParser.decode(data)
                else {
                    continue
                }
                DispatchQueue.main.async {
                    if let snapshot = envelope.payload {
                        self?.model.apply(snapshot)
                        self?.panel?.displayIfNeeded()
                        self?.model.acknowledgeRender(
                            sourceSequence: envelope.sequence,
                            receivedAtMilliseconds: receivedAtMilliseconds,
                            appliedAtMilliseconds: UInt64(Date().timeIntervalSince1970 * 1_000)
                        )
                    } else if let actionResult = envelope.actionResult {
                        self?.model.apply(actionResult)
                    }
                }
            }
            DispatchQueue.main.async {
                NSApp.terminate(nil)
            }
        }
    }

    private func startParentMonitor() {
        guard parentPID > 1 else { return }
        let timer = DispatchSource.makeTimerSource(queue: .global(qos: .utility))
        timer.schedule(deadline: .now() + 2, repeating: 2)
        timer.setEventHandler {
            if kill(self.parentPID, 0) == -1 && errno == ESRCH {
                DispatchQueue.main.async {
                    NSApp.terminate(nil)
                }
            }
        }
        timer.resume()
        parentTimer = timer
    }

    private var parentTimer: DispatchSourceTimer?
}

func islandTopBoundary(
    frameMaxY: CGFloat,
    visibleFrameMaxY: CGFloat,
    safeAreaTop: CGFloat
) -> CGFloat {
    min(visibleFrameMaxY, frameMaxY - max(0, safeAreaTop))
}

private func parentProcessID() -> pid_t {
    let arguments = CommandLine.arguments
    guard let index = arguments.firstIndex(of: "--parent-pid"),
          arguments.indices.contains(index + 1),
          let value = Int32(arguments[index + 1])
    else {
        return getppid()
    }
    return value
}

let application = NSApplication.shared
let controller = IslandApplicationController(parentPID: parentProcessID())
application.delegate = controller
application.run()
