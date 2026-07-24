// macos-ui-driver — Phase 1 of docs/elwindui_macos_gui_test_driver.md (AI-agent-drivable macOS GUI
// test CLI). Phase 1 scope only: app launch/terminate, window enumeration, per-window screenshot
// capture, and permission diagnostics ("doctor"). Accessibility-tree walking, elwindui-internal
// state introspection, and image-diff regression testing are later phases, not implemented here.
//
// Every command prints exactly one JSON object to stdout and sets the process exit code (0 on
// success, 1 on failure) — never partial/streaming output, so a caller can always just parse
// stdout as one JSON value. Diagnostic/progress text (if any) goes to stderr only.
//
// No fixed `sleep`-and-hope waiting anywhere: `launch --wait-window-timeout` and `terminate
// --timeout` both poll a real condition (a window owned by the target pid appearing; the process
// actually exiting) at a short fixed interval, bounded by an explicit caller-supplied timeout —
// see pollUntil's own doc comment.

import AppKit
import ApplicationServices
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

// MARK: - JSON output

/// Serializes `object` (a `[String: Any]` of JSON-representable values) to stdout as a single
/// line and exits with `success ? 0 : 1`. The sole exit point for every command — see this file's
/// own doc comment on why every command speaks exactly one JSON object.
func emit(success: Bool, _ fields: [String: Any] = [:]) -> Never {
    var object = fields
    object["success"] = success
    let data: Data
    do {
        data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
    } catch {
        // Serialization itself failing is a driver bug, not a caller error — still emit *something*
        // parseable rather than crashing with no output at all.
        FileHandle.standardOutput.write(
            "{\"success\":false,\"error\":\"failed to serialize JSON output\"}\n".data(using: .utf8)!
        )
        exit(1)
    }
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write("\n".data(using: .utf8)!)
    exit(success ? 0 : 1)
}

func fail(_ message: String, _ extra: [String: Any] = [:]) -> Never {
    var fields = extra
    fields["error"] = message
    emit(success: false, fields)
}

// MARK: - Argument parsing

/// Minimal hand-rolled `--flag value` / `--flag` (boolean) parser — no external dependency (e.g.
// swift-argument-parser) so this tool builds offline with plain `swift build`, no package
/// resolution network access needed.
struct Args {
    private var values: [String: String] = [:]
    private var flags: Set<String> = []
    /// Repeated `--arg X` occurrences, in order — used by `launch --arg` for the child process's
    /// own argv.
    private(set) var repeatedArg: [String] = []

    init(_ argv: [String]) {
        var i = 0
        while i < argv.count {
            let token = argv[i]
            guard token.hasPrefix("--") else {
                i += 1
                continue
            }
            let name = String(token.dropFirst(2))
            let next = i + 1 < argv.count ? argv[i + 1] : nil
            if name == "arg" {
                if let next {
                    repeatedArg.append(next)
                    i += 2
                } else {
                    i += 1
                }
                continue
            }
            if let next, !next.hasPrefix("--") {
                values[name] = next
                i += 2
            } else {
                flags.insert(name)
                i += 1
            }
        }
    }

    func string(_ name: String) -> String? { values[name] }
    func requireString(_ name: String) -> String {
        guard let v = values[name] else { fail("missing required --\(name)") }
        return v
    }
    func int(_ name: String) -> Int? { values[name].flatMap { Int($0) } }
    func double(_ name: String) -> Double? { values[name].flatMap { Double($0) } }
    func flag(_ name: String) -> Bool { flags.contains(name) }
}

/// Polls `condition` every `interval` seconds until it returns non-nil or `timeout` seconds have
/// elapsed — the sole waiting primitive every command with a "wait for X" option uses, so nothing
/// in this tool ever does a fixed blind `sleep` and assumes success.
func pollUntil<T>(timeout: Double, interval: Double = 0.1, condition: () -> T?) -> T? {
    let deadline = Date().addingTimeInterval(timeout)
    while true {
        if let value = condition() {
            return value
        }
        if Date() >= deadline {
            return nil
        }
        Thread.sleep(forTimeInterval: interval)
    }
}

// MARK: - Window enumeration (shared by list-windows, launch --wait-window, capture-window)

struct WindowInfo {
    let windowID: CGWindowID
    let ownerPID: pid_t
    let ownerName: String
    let title: String
    let layer: Int
    let x: Double
    let y: Double
    let width: Double
    let height: Double

    var jsonObject: [String: Any] {
        [
            "window_id": Int(windowID),
            "pid": Int(ownerPID),
            "owner_name": ownerName,
            "title": title,
            "layer": layer,
            "x": x, "y": y, "width": width, "height": height,
        ]
    }
}

/// All on-screen windows, in front-to-back order — `CGWindowListCopyWindowInfo` itself already
/// excludes off-screen/minimized windows via `.optionOnScreenOnly`; `.excludeDesktopElements`
/// additionally drops the desktop icons layer and similar chrome, matching the project's own
/// existing screenshot recipe (see `CLAUDE.md`'s "Taking screenshots" section, which this tool
/// supersedes for AI-agent use — humans can keep using that snippet directly).
func listOnScreenWindows() -> [WindowInfo] {
    guard
        let raw = CGWindowListCopyWindowInfo(
            [.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID
        ) as? [[String: Any]]
    else {
        return []
    }
    return raw.compactMap { w -> WindowInfo? in
        guard
            let windowID = w[kCGWindowNumber as String] as? Int,
            let ownerPID = w[kCGWindowOwnerPID as String] as? Int,
            let layer = w[kCGWindowLayer as String] as? Int,
            let bounds = w[kCGWindowBounds as String] as? [String: Any],
            let x = bounds["X"] as? Double,
            let y = bounds["Y"] as? Double,
            let width = bounds["Width"] as? Double,
            let height = bounds["Height"] as? Double
        else {
            return nil
        }
        let ownerName = w[kCGWindowOwnerName as String] as? String ?? ""
        let title = w[kCGWindowName as String] as? String ?? ""
        return WindowInfo(
            windowID: CGWindowID(windowID), ownerPID: pid_t(ownerPID), ownerName: ownerName,
            title: title, layer: layer, x: x, y: y, width: width, height: height
        )
    }
}

// MARK: - doctor

/// Reports Screen Recording / Accessibility permission state without ever *prompting* for either
/// (`CGPreflightScreenCaptureAccess`/`AXIsProcessTrusted` are both preflight-only checks, unlike
/// their `*Request*` counterparts) — safe to call from an unattended agent loop.
func cmdDoctor() -> Never {
    let screenRecording = CGPreflightScreenCaptureAccess()
    let accessibility = AXIsProcessTrusted()
    let version = ProcessInfo.processInfo.operatingSystemVersionString
    emit(
        success: true,
        [
            "screen_recording": screenRecording,
            "accessibility": accessibility,
            "macos_version": version,
        ])
}

// MARK: - launch

func cmdLaunch(_ args: Args) -> Never {
    let path = args.requireString("path")
    guard FileManager.default.isExecutableFile(atPath: path) else {
        fail("not an executable file: \(path)", ["path": path])
    }
    let process = Process()
    process.executableURL = URL(fileURLWithPath: path)
    process.arguments = args.repeatedArg
    if let cwd = args.string("cwd") {
        process.currentDirectoryURL = URL(fileURLWithPath: cwd)
    }
    do {
        try process.run()
    } catch {
        fail("failed to launch process: \(error.localizedDescription)", ["path": path])
    }
    let pid = process.processIdentifier

    var windowFields: [String: Any] = [:]
    if let timeout = args.double("wait-window-timeout") {
        let found = pollUntil(timeout: timeout) {
            listOnScreenWindows().first { $0.ownerPID == pid && $0.layer == 0 }
        }
        if let found {
            windowFields["window"] = found.jsonObject
        } else {
            windowFields["window_wait_timed_out"] = true
        }
    }

    emit(success: true, ["pid": Int(pid)].merging(windowFields) { _, new in new })
}

// MARK: - terminate

/// `SIGTERM` first, `SIGKILL` only if the process hasn't exited by `timeout` — a real exit check
/// each poll (via `kill(pid, 0)`, the standard liveness-probe idiom: signal 0 sends nothing but
/// still fails with ESRCH once the process is gone), never a blind "sleep N seconds and assume it
/// died".
func cmdTerminate(_ args: Args) -> Never {
    let pid = pid_t(args.int("pid") ?? { fail("missing or invalid --pid") }())
    let timeout = args.double("timeout") ?? 5.0

    if kill(pid, SIGTERM) != 0 && errno == ESRCH {
        emit(success: true, ["pid": Int(pid), "already_exited": true])
    }

    let exited = pollUntil(timeout: timeout) { () -> Bool? in
        kill(pid, 0) != 0 && errno == ESRCH ? true : nil
    }

    if exited == true {
        emit(success: true, ["pid": Int(pid), "terminated": true, "forced": false])
    }

    // Didn't respond to SIGTERM in time — escalate.
    _ = kill(pid, SIGKILL)
    let killedExited = pollUntil(timeout: 2.0) { () -> Bool? in
        kill(pid, 0) != 0 && errno == ESRCH ? true : nil
    }
    emit(
        success: killedExited == true,
        ["pid": Int(pid), "terminated": killedExited == true, "forced": true])
}

// MARK: - focus-window

/// AX attribute/action helpers — thin wrappers, not a general AX abstraction (Phase 2's own
/// Accessibility-tree walking will need a real one; this is just enough for `focus-window`).
func axCopyAttribute(_ element: AXUIElement, _ attribute: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success else {
        return nil
    }
    return value
}
func axBool(_ element: AXUIElement, _ attribute: String) -> Bool {
    (axCopyAttribute(element, attribute) as? Bool) ?? false
}
func axString(_ element: AXUIElement, _ attribute: String) -> String {
    (axCopyAttribute(element, attribute) as? String) ?? ""
}
func axWindows(_ appElement: AXUIElement) -> [AXUIElement] {
    guard let raw = axCopyAttribute(appElement, kAXWindowsAttribute as String) else { return [] }
    return (raw as? [AXUIElement]) ?? []
}

/// Foregrounding an app/window on macOS 14+ is a *request*, never a guarantee — see this
/// function's own extensive doc comment (transcribed from the driver's own design notes) on why
/// every step here is a two-stage request-then-verify, never a single "call X, assume success"
/// step:
///
/// 1. `NSRunningApplication.activate()` requests app activation (its return value is **not**
///    trusted as proof of success — macOS may silently decline it, e.g. because the requesting
///    process itself isn't foreground/user-attended, which is exactly the case for an AI agent's
///    own background shell).
/// 2. `AXUIElementPerformAction(window, kAXRaiseAction)` requests that specific window be raised
///    (same caveat — its return value is **not** trusted either).
/// 3. Only *observed state* counts as proof: `runningApp.isActive`, `NSWorkspace.shared.
///    frontmostApplication` naming the target pid, the target window's own `AXMain` attribute,
///    and the app's `AXFocusedWindow` attribute (`CFEqual`-compared, not just "some window") all
///    being true/matching, polled for up to `--timeout` seconds (never a single one-shot check
///    immediately after step 1/2, since activation is not synchronous).
///
/// If foregrounding can't be confirmed within the timeout, this reports `success: false` together
/// with every one of those observed values (plus the raw `activate()`/`AXRaise` return values, the
/// current actual frontmost application, the target app's `NSApplication.ActivationPolicy`, and
/// the macOS version) — **not** as a driver bug to paper over, but as a real, reportable
/// environment constraint (e.g. a sandboxed/agent shell that macOS declines to let steal
/// foreground focus from the user's actual active application — see
/// `docs/elwindui_macos_gui_test_driver_status.md` for a concrete case this was observed in). A
/// caller that needs guaranteed-stable foregrounding for CI should prefer XCUITest, which runs
/// inside the same test-runner process macOS already trusts to drive the UI, rather than fighting
/// this from an external CLI.
func cmdFocusWindow(_ args: Args) -> Never {
    let pid = pid_t(args.int("pid") ?? { fail("missing or invalid --pid") }())
    let titleContains = args.string("title")
    let timeout = args.double("timeout") ?? 3.0

    guard let runningApp = NSRunningApplication(processIdentifier: pid) else {
        fail("no running application with pid \(pid)", ["pid": Int(pid)])
    }
    let appElement = AXUIElementCreateApplication(pid)
    let windows = axWindows(appElement)
    guard !windows.isEmpty else {
        fail(
            "application has no AX windows (is Accessibility permission granted? see `doctor`)",
            baseDiagnostics(pid: pid, runningApp: runningApp))
    }
    let targetWindow: AXUIElement
    if let titleContains {
        guard
            let match = windows.first(where: {
                axString($0, kAXTitleAttribute as String).localizedCaseInsensitiveContains(
                    titleContains)
            })
        else {
            fail(
                "no AX window with title containing \"\(titleContains)\" (found: \(windows.map { axString($0, kAXTitleAttribute as String) }))",
                baseDiagnostics(pid: pid, runningApp: runningApp))
        }
        targetWindow = match
    } else {
        targetWindow = windows[0]
    }

    // Stage 1: request app activation. Stage 2: request the specific window be raised. Neither
    // return value is trusted — see this function's own doc comment.
    let activateRequested = runningApp.activate()
    let raiseStatus = AXUIElementPerformAction(targetWindow, kAXRaiseAction as CFString)

    func isFocusedWindowMatch() -> Bool {
        guard let focused = axCopyAttribute(appElement, kAXFocusedWindowAttribute as String) else {
            return false
        }
        return CFEqual(focused, targetWindow)
    }

    let confirmed = pollUntil(timeout: timeout) { () -> Bool? in
        let isActive = runningApp.isActive
        let isFrontmost = NSWorkspace.shared.frontmostApplication?.processIdentifier == pid
        let isMain = axBool(targetWindow, kAXMainAttribute as String)
        let isFocused = isFocusedWindowMatch()
        return (isActive && isFrontmost && isMain && isFocused) ? true : nil
    }

    var diagnostics = baseDiagnostics(pid: pid, runningApp: runningApp)
    diagnostics["activate_requested_ok"] = activateRequested
    diagnostics["ax_raise_status_ok"] = (raiseStatus == .success)
    diagnostics["ax_title"] = axString(targetWindow, kAXTitleAttribute as String)
    diagnostics["ax_main"] = axBool(targetWindow, kAXMainAttribute as String)
    diagnostics["ax_focused_window_matches_target"] = isFocusedWindowMatch()

    if confirmed == true {
        emit(success: true, diagnostics)
    } else {
        diagnostics["error"] =
            "could not confirm the window is actually frontmost/main/focused within \(timeout)s — activate()/AXRaise return values alone are not proof of success on macOS 14+; this may be an environment-level restriction (e.g. this process's own foreground/user-attended status) rather than an application defect — see this command's own design notes in docs/elwindui_macos_gui_test_driver_status.md"
        emit(success: false, diagnostics)
    }
}

/// Fields required by every `focus-window` outcome (success or failure) per the "record on
/// failure" list this command's design was given — included unconditionally, not just on failure,
/// since they're cheap and a caller may want them for a successful run's own audit trail too.
func baseDiagnostics(pid: pid_t, runningApp: NSRunningApplication) -> [String: Any] {
    let frontmost = NSWorkspace.shared.frontmostApplication
    return [
        "pid": Int(pid),
        "is_active": runningApp.isActive,
        "activation_policy": String(describing: runningApp.activationPolicy),
        "frontmost_application_pid": frontmost.map { Int($0.processIdentifier) } ?? -1,
        "frontmost_application_name": frontmost?.localizedName ?? frontmost?.bundleIdentifier ?? "",
        "macos_version": ProcessInfo.processInfo.operatingSystemVersionString,
    ]
}

// MARK: - list-windows

func cmdListWindows(_ args: Args) -> Never {
    var windows = listOnScreenWindows()
    if let pid = args.int("pid") {
        windows = windows.filter { $0.ownerPID == pid_t(pid) }
    }
    if let name = args.string("name") {
        windows = windows.filter { $0.ownerName.localizedCaseInsensitiveContains(name) }
    }
    emit(success: true, ["windows": windows.map { $0.jsonObject }])
}

// MARK: - capture-window

/// Captures *only* `windowID` (never the full screen — see `CLAUDE.md`'s own established
/// rationale: a full-screen capture pulls in the menu bar, desktop, and unrelated windows, wasting
/// context on anything that reads the result). `.boundsIgnoreFraming` crops to the window's actual
/// content bounds (no drop-shadow padding); `.bestResolution` captures at the display's real pixel
/// density (Retina-correct) rather than a possibly-downscaled default.
func cmdCaptureWindow(_ args: Args) -> Never {
    let windowID = CGWindowID(args.int("window-id") ?? { fail("missing or invalid --window-id") }())
    let outPath = args.requireString("out")

    guard
        let image = CGWindowListCreateImage(
            .null, .optionIncludingWindow, windowID, [.boundsIgnoreFraming, .bestResolution]
        )
    else {
        fail(
            "CGWindowListCreateImage returned nil — window may not exist, or Screen Recording permission is not granted (see `doctor`)",
            ["window_id": Int(windowID)])
    }

    let outURL = URL(fileURLWithPath: outPath)
    guard
        let destination = CGImageDestinationCreateWithURL(
            outURL as CFURL, UTType.png.identifier as CFString, 1, nil)
    else {
        fail("failed to create PNG destination at \(outPath)")
    }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else {
        fail("failed to write PNG to \(outPath)")
    }

    emit(
        success: true,
        [
            "window_id": Int(windowID),
            "path": outPath,
            "width": image.width,
            "height": image.height,
        ])
}

// MARK: - entry point

let argv = Array(CommandLine.arguments.dropFirst())
guard let command = argv.first else {
    fail(
        "usage: macos-ui-driver <doctor|launch|terminate|list-windows|capture-window|focus-window> [options]"
    )
}
let args = Args(Array(argv.dropFirst()))

switch command {
case "doctor": cmdDoctor()
case "launch": cmdLaunch(args)
case "terminate": cmdTerminate(args)
case "list-windows": cmdListWindows(args)
case "capture-window": cmdCaptureWindow(args)
case "focus-window": cmdFocusWindow(args)
default:
    fail("unknown command: \(command)")
}
