// macos-ui-driver — Phase 1 + Phase 2 of docs/status/tooling_status.md (AI-agent-drivable
// macOS GUI test CLI). Phase 1: app launch/terminate, window enumeration, per-window screenshot
// capture, and permission diagnostics ("doctor"). Phase 2: Accessibility-tree walking
// (`dump-tree`/`find`) and control interaction (`set-focus`/`click`/`point-click`/`drag`/`resize`/
// `type-text`/`press-key`/`wait-for`) — driver-side only, see docs/status/tooling_status.md for
// scope notes.
// elwindui-internal state introspection and image-diff regression testing are later phases, not
// implemented here.
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
import Carbon.HIToolbox
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
    // Without this, the child inherits this process's own stdout (the fd behind our one-line JSON
    // response) and, being a long-lived GUI app, keeps its write end open indefinitely — so a
    // caller reading our output via a shell pipe or `$(...)` blocks until the *launched app* exits,
    // not until we do. See docs/status/tooling_status.md's "呼び出し側の既知の落とし穴"
    // for the hang this caused in practice. `.standardError` gets the same treatment for symmetry.
    process.standardOutput = FileHandle.nullDevice
    process.standardError = FileHandle.nullDevice
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
/// `docs/status/tooling_status.md` for a concrete case this was observed in). A
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
            "could not confirm the window is actually frontmost/main/focused within \(timeout)s — activate()/AXRaise return values alone are not proof of success on macOS 14+; this may be an environment-level restriction (e.g. this process's own foreground/user-attended status) rather than an application defect — see this command's own design notes in docs/status/tooling_status.md"
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

/// Verifies the observed foreground state before a coordinate-based input is sent. AX controls
/// expose their own geometry, but custom elwindui surfaces such as Docking headers do not; keeping
/// this check in the coordinate commands prevents a successful CGEventPost from being mistaken for
/// input delivered to the intended app.
func requireForeground(pid: pid_t, window: AXUIElement) -> [String: Any] {
    guard let runningApp = NSRunningApplication(processIdentifier: pid) else {
        fail("no running application with pid \(pid)", ["pid": Int(pid)])
    }
    let appElement = AXUIElementCreateApplication(pid)
    guard let focused = axCopyAttribute(appElement, kAXFocusedWindowAttribute as String) else {
        fail(
            "target application has no focused AX window",
            baseDiagnostics(pid: pid, runningApp: runningApp))
    }
    let isActive = runningApp.isActive
    let isFrontmost = NSWorkspace.shared.frontmostApplication?.processIdentifier == pid
    let isMain = axBool(window, kAXMainAttribute as String)
    let isFocused = CFEqual(focused, window)
    var diagnostics = baseDiagnostics(pid: pid, runningApp: runningApp)
    diagnostics["ax_main"] = isMain
    diagnostics["ax_focused_window_matches_target"] = isFocused
    guard isActive && isFrontmost && isMain && isFocused else {
        diagnostics["error"] =
            "target window is not confirmed frontmost/main/focused; run focus-window first"
        fail("coordinate input requires a confirmed foreground target", diagnostics)
    }
    return diagnostics
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

// MARK: - Phase 2: AX tree model

/// A richer per-element snapshot than the Phase 1 `axCopyAttribute`/`axBool`/`axString` quartet
/// (which stays as-is for `focus-window`'s own 2-attribute reads — not worth churning verified
/// code). Every Phase 2 command builds and serializes these instead of touching raw `AXUIElement`
/// attributes ad hoc.
struct AXElement {
    let element: AXUIElement
    let role: String
    let subrole: String
    let title: String
    let value: Any?
    let identifier: String
    let position: CGPoint?
    let size: CGSize?
    let enabled: Bool
    let focused: Bool
    let childCount: Int

    init(_ element: AXUIElement) {
        self.element = element
        role = axString(element, kAXRoleAttribute as String)
        subrole = axString(element, kAXSubroleAttribute as String)
        title = axString(element, kAXTitleAttribute as String)
        identifier = axString(element, kAXIdentifierAttribute as String)
        value = axJSONValue(axCopyAttribute(element, kAXValueAttribute as String))
        position = axPoint(element, kAXPositionAttribute as String)
        size = axSize(element, kAXSizeAttribute as String)
        enabled = (axCopyAttribute(element, kAXEnabledAttribute as String) as? Bool) ?? true
        focused = axBool(element, kAXFocusedAttribute as String)
        childCount = (axCopyAttribute(element, kAXChildrenAttribute as String) as? [AXUIElement])?.count ?? 0
    }

    func jsonObject(includeChildren children: [[String: Any]]? = nil) -> [String: Any] {
        var obj: [String: Any] = [
            "role": role, "subrole": subrole, "title": title, "identifier": identifier,
            "enabled": enabled, "focused": focused, "value": value ?? NSNull(), "child_count": childCount,
        ]
        obj["position"] = position.map { ["x": $0.x, "y": $0.y] } ?? NSNull()
        obj["size"] = size.map { ["width": $0.width, "height": $0.height] } ?? NSNull()
        if let children { obj["children"] = children }
        return obj
    }
}

/// Decodes an `AXValue`-wrapped `CGPoint` attribute (e.g. `kAXPositionAttribute`) — Phase 1 never
/// needed position/size, so this decode step didn't exist before Phase 2.
func axPoint(_ element: AXUIElement, _ attribute: String) -> CGPoint? {
    guard let raw = axCopyAttribute(element, attribute), CFGetTypeID(raw) == AXValueGetTypeID() else {
        return nil
    }
    let axValue = raw as! AXValue
    guard AXValueGetType(axValue) == .cgPoint else { return nil }
    var point = CGPoint.zero
    return AXValueGetValue(axValue, .cgPoint, &point) ? point : nil
}

/// Decodes an `AXValue`-wrapped `CGSize` attribute (e.g. `kAXSizeAttribute`).
func axSize(_ element: AXUIElement, _ attribute: String) -> CGSize? {
    guard let raw = axCopyAttribute(element, attribute), CFGetTypeID(raw) == AXValueGetTypeID() else {
        return nil
    }
    let axValue = raw as! AXValue
    guard AXValueGetType(axValue) == .cgSize else { return nil }
    var size = CGSize.zero
    return AXValueGetValue(axValue, .cgSize, &size) ? size : nil
}

/// Best-effort JSON coercion of a raw `kAXValueAttribute` read: String/Bool/NSNumber map directly;
/// anything else falls back to `String(describing:)` (distinguishable by callers, since it won't
/// parse as the expected type).
///
/// Bool and NSNumber are distinguished by `CFGetTypeID`, not Swift's `as? Bool`/`as? NSNumber`
/// casts — `NSNumber`'s Bool bridging is permissive enough that `(0 as NSNumber) as? Bool`
/// succeeds and yields `false` (likewise `1` -> `true`), which previously misreported a genuinely
/// numeric `AXValue` (e.g. an `AXSlider` sitting at exactly its minimum or maximum) as a boolean
/// whenever `as? Bool` was tried before `as? NSNumber` — confirmed empirically via `Slider`'s own
/// `value` hitting `0.0`. `CFBooleanGetTypeID()`/`CFNumberGetTypeID()` are the real, unambiguous
/// CoreFoundation type tags underneath, so checking those first removes the ambiguity entirely.
func axJSONValue(_ raw: CFTypeRef?) -> Any? {
    guard let raw else { return nil }
    if let s = raw as? String { return s }
    let typeID = CFGetTypeID(raw)
    if typeID == CFBooleanGetTypeID() {
        return CFBooleanGetValue((raw as! CFBoolean))
    }
    if typeID == CFNumberGetTypeID() {
        return raw as! NSNumber
    }
    return String(describing: raw)
}

/// String form of an `AXElement.value` for equality/comparison purposes (`click`'s before/after
/// diff, `wait-for`'s `value-equals` condition) — avoids `Any?`-vs-`Any?` comparison pitfalls.
func axValueDescription(_ v: Any?) -> String {
    guard let v else { return "" }
    return "\(v)"
}

struct AXNode {
    let axElement: AXElement
    let children: [AXNode]

    var jsonObject: [String: Any] { axElement.jsonObject(includeChildren: children.map { $0.jsonObject }) }

    /// Pre-order flatten — used by every selector-matching command (`find`/`click`/etc.) to search
    /// the whole subtree as a flat list.
    func flattened() -> [AXNode] { [self] + children.flatMap { $0.flattened() } }
}

/// Depth-first, pre-order recursive walk of `kAXChildrenAttribute`. `maxDepth` is the primary
/// cycle/pathological-tree guard; `maxNodes` (checked via the shared `inout` counter) independently
/// bounds a very wide/bushy tree that a depth cap alone wouldn't catch. AX trees for a single
/// on-screen app window are not expected to be cyclic — this is a cheap, unconditional safety net,
/// not a targeted fix for an observed cycle.
func buildAXTree(_ element: AXUIElement, depth: Int, maxDepth: Int, nodeCount: inout Int, maxNodes: Int)
    -> AXNode
{
    nodeCount += 1
    let info = AXElement(element)
    guard depth < maxDepth, nodeCount < maxNodes else { return AXNode(axElement: info, children: []) }
    let rawChildren = (axCopyAttribute(element, kAXChildrenAttribute as String) as? [AXUIElement]) ?? []
    let children = rawChildren.map {
        buildAXTree($0, depth: depth + 1, maxDepth: maxDepth, nodeCount: &nodeCount, maxNodes: maxNodes)
    }
    return AXNode(axElement: info, children: children)
}

// MARK: - Phase 2: element selector

/// Shared by `find`/`click`/`type-text`/`press-key`/`set-focus` — the same four selector flags
/// everywhere, resolved consistently by `filterNodes`/`resolveElement` below.
struct ElementSelector {
    let role: String?
    let title: String?
    let titleContains: String?
    let identifier: String?
    let index: Int?

    init(_ args: Args) {
        role = args.string("role")
        title = args.string("title")
        titleContains = args.string("title-contains")
        identifier = args.string("identifier")
        index = args.int("index")
    }

    var isEmpty: Bool { role == nil && title == nil && titleContains == nil && identifier == nil }
}

/// Pure, non-failing filter — used directly by `find`/`wait-for` (which must observe "0 matches so
/// far" without aborting) and internally by `resolveElement` below.
func filterNodes(_ nodes: [AXNode], matching s: ElementSelector) -> [AXNode] {
    nodes.filter { node in
        if let role = s.role, node.axElement.role.caseInsensitiveCompare(role) != .orderedSame {
            return false
        }
        if let title = s.title, node.axElement.title.caseInsensitiveCompare(title) != .orderedSame {
            return false
        }
        if let tc = s.titleContains, !node.axElement.title.localizedCaseInsensitiveContains(tc) {
            return false
        }
        if let id = s.identifier, node.axElement.identifier != id { return false }
        return true
    }
}

/// Strict single-match requirement used by `click`/`type-text`/`press-key`/`set-focus` — these
/// commands cause a real side effect on a live element, so they never guess among multiple matches
/// and never silently no-op on zero. `Never`-returning on failure, matching `Args.requireString`'s
/// own idiom.
func resolveElement(in root: AXUIElement, selector: ElementSelector, maxDepth: Int) -> AXUIElement {
    guard !selector.isEmpty else {
        fail("at least one of --role/--title/--title-contains/--identifier is required")
    }
    var nodeCount = 0
    let tree = buildAXTree(root, depth: 0, maxDepth: maxDepth, nodeCount: &nodeCount, maxNodes: 2000)
    let matches = filterNodes(tree.flattened(), matching: selector)
    if let index = selector.index {
        guard matches.indices.contains(index) else {
            fail("--index \(index) out of range (\(matches.count) matches)", ["match_count": matches.count])
        }
        return matches[index].axElement.element
    }
    guard matches.count == 1 else {
        fail(
            matches.isEmpty
                ? "no element matched selector"
                : "\(matches.count) elements matched selector — add --index or narrow the selector",
            [
                "match_count": matches.count,
                "matches": Array(matches.prefix(20).map { $0.axElement.jsonObject() }),
            ])
    }
    return matches[0].axElement.element
}

// MARK: - Phase 2: window resolution

/// Core geometry+title matching algorithm, used only when an app has more than one AX window (the
/// common single-window case short-circuits elsewhere with zero ambiguity risk). Deliberately no
/// private API (`_AXUIElementGetWindow`) — matches the already-existing `CGWindowID` (from
/// `listOnScreenWindows()`, shared with `list-windows`/`capture-window`) against `axWindows`
/// entries by `kAXTitleAttribute` + `kAXPositionAttribute`/`kAXSizeAttribute`, within a small
/// epsilon absorbing CG-vs-AX rounding differences. Tradeoff: two on-screen windows with identical
/// title AND identical frame would collide — judged acceptable since this project's example apps
/// are single-window (this path exists for forward compatibility, not because it's exercised
/// today).
func resolveWindow(
    pid: pid_t, appElement: AXUIElement, windowID: CGWindowID?, windowTitleContains: String?
) -> AXUIElement? {
    let windows = axWindows(appElement)
    if let windowTitleContains {
        return windows.first {
            axString($0, kAXTitleAttribute as String).localizedCaseInsensitiveContains(
                windowTitleContains)
        }
    }
    guard let windowID else { return windows.first }
    if windows.count == 1 { return windows[0] }
    guard let info = listOnScreenWindows().first(where: { $0.windowID == windowID && $0.ownerPID == pid })
    else { return nil }
    let epsilon = 2.0
    return windows.first { candidate in
        let title = axString(candidate, kAXTitleAttribute as String)
        guard title == info.title else { return false }
        guard let pos = axPoint(candidate, kAXPositionAttribute as String),
            let size = axSize(candidate, kAXSizeAttribute as String)
        else { return false }
        return abs(pos.x - info.x) < epsilon && abs(pos.y - info.y) < epsilon
            && abs(size.width - info.width) < epsilon && abs(size.height - info.height) < epsilon
    }
}

/// Shared entry point every Phase 2 command calls to turn `--window-id`/`--window-title` into a
/// target `AXUIElement` window. Stricter than `focus-window`'s unconditional `windows[0]` fallback:
/// with no qualifier and more than one AX window, this `fail()`s rather than guessing, since these
/// commands cause real side effects.
func resolveTargetWindow(_ args: Args, pid: pid_t, appElement: AXUIElement) -> AXUIElement {
    let windows = axWindows(appElement)
    guard !windows.isEmpty else {
        fail(
            "application has no AX windows (is Accessibility permission granted? see `doctor`)",
            ["pid": Int(pid)])
    }
    let windowID = args.int("window-id").map { CGWindowID($0) }
    let windowTitle = args.string("window-title")
    if windowID == nil && windowTitle == nil {
        if windows.count == 1 { return windows[0] }
        fail(
            "multiple AX windows for pid \(pid); pass --window-id or --window-title",
            ["window_titles": windows.map { axString($0, kAXTitleAttribute as String) }])
    }
    guard
        let resolved = resolveWindow(
            pid: pid, appElement: appElement, windowID: windowID, windowTitleContains: windowTitle)
    else {
        fail(
            "could not resolve target window",
            ["window_titles": windows.map { axString($0, kAXTitleAttribute as String) }])
    }
    return resolved
}

/// Shared setup every Phase 2 command performs first: validate `--pid`, get the running app's AX
/// application element, resolve the target window from `--window-id`/`--window-title`.
func resolveContext(_ args: Args) -> (pid: pid_t, appElement: AXUIElement, window: AXUIElement) {
    let pid = pid_t(args.int("pid") ?? { fail("missing or invalid --pid") }())
    guard NSRunningApplication(processIdentifier: pid) != nil else {
        fail("no running application with pid \(pid)", ["pid": Int(pid)])
    }
    let appElement = AXUIElementCreateApplication(pid)
    let window = resolveTargetWindow(args, pid: pid, appElement: appElement)
    return (pid, appElement, window)
}

struct MouseButtonEvents {
    let down: CGEventType
    let dragged: CGEventType
    let up: CGEventType
    let button: CGMouseButton
}

func mouseButtonEvents(_ raw: String) -> MouseButtonEvents? {
    switch raw.lowercased() {
    case "left":
        return MouseButtonEvents(
            down: .leftMouseDown, dragged: .leftMouseDragged, up: .leftMouseUp, button: .left)
    case "right":
        return MouseButtonEvents(
            down: .rightMouseDown, dragged: .rightMouseDragged, up: .rightMouseUp, button: .right)
    default:
        return nil
    }
}

func requirePointInsideWindow(_ point: CGPoint, window: AXUIElement) {
    guard let origin = axPoint(window, kAXPositionAttribute as String),
        let size = axSize(window, kAXSizeAttribute as String),
        point.x >= origin.x,
        point.y >= origin.y,
        point.x <= origin.x + size.width,
        point.y <= origin.y + size.height
    else {
        fail(
            "coordinate is outside the target window",
            ["point": ["x": point.x, "y": point.y]])
    }
}

/// Sends a real mouse click at an explicit screen coordinate. This is intentionally separate from
/// `click`, which resolves an AX element, because custom elwindui surfaces are often not exposed in
/// the Accessibility tree. The foreground and target-window checks keep this useful for evidence,
/// rather than turning it into an unrestricted global click primitive.
func postPointClick(at point: CGPoint, button: MouseButtonEvents, pause: Double) -> Bool {
    let down = CGEvent(
        mouseEventSource: nil, mouseType: button.down, mouseCursorPosition: point,
        mouseButton: button.button)
    let up = CGEvent(
        mouseEventSource: nil, mouseType: button.up, mouseCursorPosition: point,
        mouseButton: button.button)
    guard let down, let up else { return false }
    down.post(tap: .cghidEventTap)
    if pause > 0.0 {
        Thread.sleep(forTimeInterval: pause)
    }
    up.post(tap: .cghidEventTap)
    return true
}

/// Sends a real press-move-release gesture at screen coordinates, preserving intermediate events
/// for drag-preview and splitter tracking tests. The caller can capture the target window from a
/// second process while this command is running to inspect mid-drag state.
func postMouseDrag(
    from start: CGPoint,
    to end: CGPoint,
    button: MouseButtonEvents,
    steps: Int,
    duration: Double
) -> Bool {
    let move = CGEvent(
        mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: start, mouseButton: button.button)
    let down = CGEvent(
        mouseEventSource: nil, mouseType: button.down, mouseCursorPosition: start,
        mouseButton: button.button)
    guard let move, let down else { return false }
    move.post(tap: .cghidEventTap)
    down.post(tap: .cghidEventTap)

    let interval = duration / Double(steps)
    for step in 1...steps {
        if interval > 0.0 {
            Thread.sleep(forTimeInterval: interval)
        }
        let fraction = Double(step) / Double(steps)
        let point = CGPoint(
            x: start.x + (end.x - start.x) * fraction,
            y: start.y + (end.y - start.y) * fraction)
        guard
            let dragged = CGEvent(
                mouseEventSource: nil, mouseType: button.dragged, mouseCursorPosition: point,
                mouseButton: button.button)
        else { return false }
        dragged.post(tap: .cghidEventTap)
    }

    guard
        let up = CGEvent(
            mouseEventSource: nil, mouseType: button.up, mouseCursorPosition: end,
            mouseButton: button.button)
    else { return false }
    up.post(tap: .cghidEventTap)
    return true
}

/// Shared mouse-click synthesis used by `click --via mouse`, `type-text --focus-via click`, and
/// `press-key --focus-via click` — a real `CGEventPost` down/up pair at `.cghidEventTap`, computed
/// from the element's own `AXPosition`/`AXSize`. Deliberately not `postToPid`-targeted: the whole
/// point is to exercise real hit-testing/focus routing, which targeted delivery would bypass.
///
/// `xFraction` (0.0 = left edge, 1.0 = right edge, default 0.5 = center) picks where along the
/// element's own width the click lands — a plain click still can't drag, but for a control whose
/// value follows click position directly (`AXSlider` and similar single-click-to-position widgets)
/// this is enough to land on an arbitrary value without needing a real drag gesture. Vertical
/// position is always the element's own vertical center; there is no `yFraction` counterpart, since
/// every control this targets today is horizontal (`click`'s own doc comment).
///
/// Returns the click point if the element had position/size to click, else `nil`.
@discardableResult
func synthesizeClick(on element: AXUIElement, xFraction: Double = 0.5, button: String = "left") -> CGPoint? {
    guard let pos = axPoint(element, kAXPositionAttribute as String),
        let size = axSize(element, kAXSizeAttribute as String)
    else { return nil }
    let point = CGPoint(x: pos.x + size.width * xFraction, y: pos.y + size.height / 2)
    let downType: CGEventType = (button == "right") ? .rightMouseDown : .leftMouseDown
    let upType: CGEventType = (button == "right") ? .rightMouseUp : .leftMouseUp
    let cgButton: CGMouseButton = (button == "right") ? .right : .left
    let down = CGEvent(
        mouseEventSource: nil, mouseType: downType, mouseCursorPosition: point, mouseButton: cgButton)
    let up = CGEvent(
        mouseEventSource: nil, mouseType: upType, mouseCursorPosition: point, mouseButton: cgButton)
    down?.post(tap: .cghidEventTap)
    Thread.sleep(forTimeInterval: 0.05)
    up?.post(tap: .cghidEventTap)
    return point
}

// MARK: - dump-tree

func cmdDumpTree(_ args: Args) -> Never {
    let (pid, _, window) = resolveContext(args)
    let maxDepth = args.int("max-depth") ?? 40
    var nodeCount = 0
    let tree = buildAXTree(window, depth: 0, maxDepth: maxDepth, nodeCount: &nodeCount, maxNodes: 2000)
    emit(
        success: true,
        [
            "pid": Int(pid),
            "node_count": nodeCount,
            "truncated": nodeCount >= 2000,
            "root": tree.jsonObject,
        ])
}

// MARK: - find

func cmdFind(_ args: Args) -> Never {
    let (pid, _, window) = resolveContext(args)
    let maxDepth = args.int("max-depth") ?? 40
    let selector = ElementSelector(args)
    var nodeCount = 0
    let tree = buildAXTree(window, depth: 0, maxDepth: maxDepth, nodeCount: &nodeCount, maxNodes: 2000)
    let matches = filterNodes(tree.flattened(), matching: selector)
    emit(
        success: true,
        [
            "pid": Int(pid),
            "match_count": matches.count,
            "matches": matches.map { $0.axElement.jsonObject() },
        ])
}

// MARK: - set-focus

/// Directly requests keyboard focus via `AXUIElementSetAttributeValue(kAXFocusedAttribute)`,
/// bypassing mouse-based hit-testing entirely. Exists specifically to let a caller distinguish
/// "click doesn't focus this control" (a mouse/hit-test/first-responder bug) from "nothing can put
/// focus on this control at all" (a deeper wiring bug) by trying both `click` and `set-focus`
/// independently against the identical selector — see this driver's real-machine finding against
/// `examples/controls-demo`'s `TextBox`, recorded in `docs/status/tooling_status.md`.
/// Same request-then-verify idiom as `focus-window`: the `AXUIElementSetAttributeValue` return code
/// is recorded but not trusted as proof; only a re-read of `AXFocused` counts.
func cmdSetFocus(_ args: Args) -> Never {
    let (pid, _, window) = resolveContext(args)
    let maxDepth = args.int("max-depth") ?? 40
    let selector = ElementSelector(args)
    let timeout = args.double("timeout") ?? 1.0
    let element = resolveElement(in: window, selector: selector, maxDepth: maxDepth)

    let before = AXElement(element)
    let status = AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue)
    let confirmed =
        pollUntil(timeout: timeout) { () -> Bool? in
            axBool(element, kAXFocusedAttribute as String) ? true : nil
        } == true
    let after = AXElement(element)

    emit(
        success: confirmed,
        [
            "pid": Int(pid),
            "set_attribute_status_ok": status == .success,
            "focus_confirmed": confirmed,
            "before": before.jsonObject(),
            "after": after.jsonObject(),
        ])
}

// MARK: - click

/// Real user-facing click, faithfully synthesized via `synthesizeClick` (real `CGEventPost` at
/// `.cghidEventTap` — the same tap Accessibility trust, reported by `doctor`, already gates, so no
/// new permission story). `--via ax-press` performs `AXUIElementPerformAction(kAXPressAction)`
/// instead, for elements where a synthetic mouse event is unnecessary (plain buttons); `--via
/// ax-increment`/`ax-decrement` likewise perform `kAXIncrementAction`/`kAXDecrementAction`, the
/// step-based counterpart `AXSlider`/`AXStepper`-family elements expose in place of `kAXPressAction`
/// (which they don't support at all — confirmed empirically, `ax_press_status_ok: false`). All four
/// modes exist specifically so a caller can trial whichever independently against the same selector.
///
/// `--fraction <0.0-1.0>` (mouse only) picks where along the element's own width the synthesized
/// click lands, instead of always its center — see `synthesizeClick`'s own `xFraction` doc comment
/// for why this alone (no real drag) is enough to exercise a single-click-to-position control like
/// `AXSlider`.
///
/// There is no universal AX signal for "did the click semantically succeed" — clicking a button vs.
/// a text field means different things. So `click` reports a before/after diff (`changed.focused`,
/// `changed.value`) as diagnostic data for the caller to interpret, rather than guessing a pass/fail
/// itself; `success` here reflects only "the element was found and the event was sent without
/// error".
func cmdClick(_ args: Args) -> Never {
    let (pid, _, window) = resolveContext(args)
    let maxDepth = args.int("max-depth") ?? 40
    let selector = ElementSelector(args)
    let timeout = args.double("timeout") ?? 1.0
    let via = args.string("via") ?? "mouse"
    let element = resolveElement(in: window, selector: selector, maxDepth: maxDepth)

    let before = AXElement(element)
    var fields: [String: Any] = ["pid": Int(pid), "via": via]

    switch via {
    case "mouse":
        let fraction = args.double("fraction") ?? 0.5
        let button = args.string("button") ?? "left"
        guard fraction >= 0.0, fraction <= 1.0 else {
            fail("--fraction must be within 0.0..=1.0, got \(fraction)")
        }
        guard let point = synthesizeClick(on: element, xFraction: fraction, button: button) else {
            fail(
                "element has no position/size — cannot compute click point",
                ["element": before.jsonObject()])
        }
        fields["click_point"] = ["x": point.x, "y": point.y]
        fields["fraction"] = fraction
        fields["button"] = button
    case "ax-press":
        let status = AXUIElementPerformAction(element, kAXPressAction as CFString)
        fields["ax_press_status_ok"] = (status == .success)
    case "ax-increment":
        let status = AXUIElementPerformAction(element, kAXIncrementAction as CFString)
        fields["ax_increment_status_ok"] = (status == .success)
    case "ax-decrement":
        let status = AXUIElementPerformAction(element, kAXDecrementAction as CFString)
        fields["ax_decrement_status_ok"] = (status == .success)
    default:
        fail("unknown --via \(via) (expected mouse, ax-press, ax-increment, or ax-decrement)")
    }

    // No universal "click landed" signal exists — poll briefly for *any* observable change, then
    // report before/after regardless, per this function's own doc comment.
    _ = pollUntil(timeout: timeout) { () -> Bool? in
        AXElement(element).focused != before.focused ? true : nil
    }
    let after = AXElement(element)

    fields["before"] = before.jsonObject()
    fields["after"] = after.jsonObject()
    fields["changed"] = [
        "focused": after.focused != before.focused,
        "value": axValueDescription(before.value) != axValueDescription(after.value),
    ]
    emit(success: true, fields)
}

// MARK: - point-click

/// Clicks an explicit screen coordinate after resolving and verifying the target AX window. This
/// is the coordinate counterpart to `click` for custom controls that intentionally have no AX node.
func cmdPointClick(_ args: Args) -> Never {
    let (pid, _, window) = resolveContext(args)
    guard let x = args.double("x"), let y = args.double("y"), x.isFinite, y.isFinite else {
        fail("point-click requires finite --x and --y")
    }
    let point = CGPoint(x: x, y: y)
    let buttonName = args.string("button") ?? "left"
    guard let button = mouseButtonEvents(buttonName) else {
        fail("unknown --button \(buttonName) (expected left or right)")
    }
    let pause = args.double("pause") ?? 0.05
    guard pause.isFinite, pause >= 0.0 else {
        fail("--pause must be a finite non-negative number")
    }
    let foreground = requireForeground(pid: pid, window: window)
    requirePointInsideWindow(point, window: window)
    guard postPointClick(at: point, button: button, pause: pause) else {
        fail("failed to create point-click events", foreground)
    }
    var fields = foreground
    fields["point"] = ["x": point.x, "y": point.y]
    fields["button"] = buttonName.lowercased()
    fields["pause_seconds"] = pause
    emit(success: true, fields)
}

// MARK: - drag

/// Drags between explicit screen coordinates after resolving and verifying the target AX window.
/// A caller can run this for several seconds and capture the window from another process while the
/// command is in flight to inspect a real preview or splitter mid-drag. By default both endpoints
/// must be inside the target window. `--allow-end-outside-window` is an explicit escape hatch for a
/// verified cross-window gesture (for example, returning a native floating host to the main host);
/// the press point remains protected by the normal in-window check.
func cmdDrag(_ args: Args) -> Never {
    guard
        let startX = args.double("start-x"),
        let startY = args.double("start-y"),
        let endX = args.double("end-x"),
        let endY = args.double("end-y"),
        startX.isFinite,
        startY.isFinite,
        endX.isFinite,
        endY.isFinite
    else {
        fail("drag requires finite --start-x/--start-y/--end-x/--end-y")
    }
    let (pid, _, window) = resolveContext(args)
    let start = CGPoint(x: startX, y: startY)
    let end = CGPoint(x: endX, y: endY)
    let buttonName = args.string("button") ?? "left"
    guard let button = mouseButtonEvents(buttonName) else {
        fail("unknown --button \(buttonName) (expected left or right)")
    }
    let steps = args.int("steps") ?? 20
    guard (1...10_000).contains(steps) else {
        fail("--steps must be within 1...10000")
    }
    let duration = args.double("duration") ?? 0.5
    guard duration.isFinite, duration >= 0.0 else {
        fail("--duration must be a finite non-negative number")
    }

    let foreground = requireForeground(pid: pid, window: window)
    requirePointInsideWindow(start, window: window)
    if !args.flag("allow-end-outside-window") {
        requirePointInsideWindow(end, window: window)
    }
    guard postMouseDrag(from: start, to: end, button: button, steps: steps, duration: duration) else {
        fail("failed to create drag events", foreground)
    }
    var fields = foreground
    fields["start"] = ["x": start.x, "y": start.y]
    fields["end"] = ["x": end.x, "y": end.y]
    fields["button"] = buttonName.lowercased()
    fields["steps"] = steps
    fields["duration_seconds"] = duration
    emit(success: true, fields)
}

// MARK: - resize

/// Resizes an AX window through the real lower-right resize handle. This is a specialized
/// counterpart to `drag`: it derives the grab point from the window's current AX bounds, so a
/// caller does not have to guess title-bar/frame coordinates, and it verifies that AppKit exposed
/// a changed size after the gesture. The end point may be outside the original bounds when the
/// requested delta grows the window; only the grab point needs to be inside the target window.
func cmdResize(_ args: Args) -> Never {
    let deltaWidth = args.double("delta-width") ?? 0.0
    let deltaHeight = args.double("delta-height") ?? 0.0
    guard deltaWidth.isFinite, deltaHeight.isFinite else {
        fail("resize requires finite --delta-width and --delta-height")
    }
    guard deltaWidth != 0.0 || deltaHeight != 0.0 else {
        fail("resize requires a non-zero --delta-width or --delta-height")
    }

    let (pid, _, window) = resolveContext(args)
    let steps = args.int("steps") ?? 20
    guard (1...10_000).contains(steps) else {
        fail("--steps must be within 1...10000")
    }
    let duration = args.double("duration") ?? 0.5
    guard duration.isFinite, duration >= 0.0 else {
        fail("--duration must be a finite non-negative number")
    }
    let timeout = args.double("timeout") ?? 1.0
    guard timeout.isFinite, timeout >= 0.0 else {
        fail("--timeout must be a finite non-negative number")
    }

    let foreground = requireForeground(pid: pid, window: window)
    guard let origin = axPoint(window, kAXPositionAttribute as String),
        let size = axSize(window, kAXSizeAttribute as String),
        size.width > 0.0,
        size.height > 0.0
    else {
        fail("target window has no usable AX position/size", foreground)
    }

    let start = CGPoint(
        x: origin.x + max(size.width - 2.0, 0.0),
        y: origin.y + max(size.height - 2.0, 0.0))
    let end = CGPoint(x: start.x + deltaWidth, y: start.y + deltaHeight)
    requirePointInsideWindow(start, window: window)
    guard postMouseDrag(
        from: start, to: end, button: mouseButtonEvents("left")!, steps: steps, duration: duration)
    else {
        fail("failed to create resize events", foreground)
    }

    let changed = pollUntil(timeout: timeout) { () -> Bool? in
        guard let current = axSize(window, kAXSizeAttribute as String) else { return nil }
        return current.width != size.width || current.height != size.height ? true : nil
    } == true
    let after = axSize(window, kAXSizeAttribute as String)
    var fields = foreground
    fields["before"] = ["width": size.width, "height": size.height]
    fields["after"] = after.map { ["width": $0.width, "height": $0.height] } as Any
    fields["delta"] = ["width": deltaWidth, "height": deltaHeight]
    fields["start"] = ["x": start.x, "y": start.y]
    fields["end"] = ["x": end.x, "y": end.y]
    fields["steps"] = steps
    fields["duration_seconds"] = duration
    fields["changed"] = changed
    emit(success: changed, fields)
}

// MARK: - type-text

/// Synthesizes real keyboard input character-by-character (not one bulk
/// `CGEventKeyboardSetUnicodeString` call for the whole string) to mimic real keystroke cadence and
/// reliably exercise `elwindui-backend-appkit`'s live change-notification delegate the same way an
/// actual keypress would, rather than risking it being treated like a paste. `virtualKey: 0` +
/// `keyboardSetUnicodeString` is the standard idiom for injecting arbitrary Unicode text independent
/// of keyboard layout.
///
/// `success` is gated on BOTH the requested focus step being verified (when `--focus-via != none`)
/// AND the post-typing value matching what was expected — a true request-then-verify command, and
/// the most decisive tool for diagnosing whether a text control's focus/input wiring actually works.
func cmdTypeText(_ args: Args) -> Never {
    let (pid, _, window) = resolveContext(args)
    let maxDepth = args.int("max-depth") ?? 40
    let selector = ElementSelector(args)
    let text = args.requireString("text")
    let clear = args.flag("clear")
    let focusVia = args.string("focus-via") ?? "ax-attribute"
    let keyDelay = args.double("key-delay") ?? 0.02
    let timeout = args.double("timeout") ?? 1.0
    let element = resolveElement(in: window, selector: selector, maxDepth: maxDepth)

    var focusConfirmed = true
    if focusVia != "none" {
        switch focusVia {
        case "ax-attribute":
            _ = AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue)
        case "click":
            synthesizeClick(on: element)
        default:
            fail("unknown --focus-via \(focusVia) (expected ax-attribute, click, or none)")
        }
        focusConfirmed =
            pollUntil(timeout: timeout) { () -> Bool? in
                axBool(element, kAXFocusedAttribute as String) ? true : nil
            } == true
        guard focusConfirmed else {
            fail(
                "could not confirm focus via --focus-via \(focusVia) within \(timeout)s — typing into unconfirmed focus is not meaningful",
                [
                    "pid": Int(pid), "focus_confirmed": false,
                    "element": AXElement(element).jsonObject(),
                ])
        }
    }

    let beforeValue = axString(element, kAXValueAttribute as String)
    if clear {
        _ = AXUIElementSetAttributeValue(element, kAXValueAttribute as CFString, "" as CFString)
    }

    let src = CGEventSource(stateID: .hidSystemState)
    for ch in text {
        var utf16 = Array(String(ch).utf16)
        let down = CGEvent(keyboardEventSource: src, virtualKey: 0, keyDown: true)
        let up = CGEvent(keyboardEventSource: src, virtualKey: 0, keyDown: false)
        down?.keyboardSetUnicodeString(stringLength: utf16.count, unicodeString: &utf16)
        down?.post(tap: .cghidEventTap)
        up?.post(tap: .cghidEventTap)
        Thread.sleep(forTimeInterval: keyDelay)
    }

    let afterValue = axString(element, kAXValueAttribute as String)
    let expected = clear ? text : beforeValue + text
    let matches = afterValue == expected || afterValue.hasSuffix(text)

    emit(
        success: focusConfirmed && matches,
        [
            "pid": Int(pid),
            "focus_confirmed": focusConfirmed,
            "before_value": beforeValue,
            "after_value": afterValue,
            "value_matches_expected": matches,
            "element": AXElement(element).jsonObject(),
        ])
}

// MARK: - press-key

/// Named-key → virtual keycode table using `Carbon.HIToolbox` constants — a system framework
/// already bundled with the macOS SDK/Xcode command line tools, so importing it does not violate
/// this tool's "no external SwiftPM dependency" constraint (that constraint is about package
/// resolution, not first-party frameworks; Phase 1 already imports `AppKit`/`ApplicationServices`
/// on the same basis).
let namedVirtualKeys: [String: CGKeyCode] = [
    "enter": CGKeyCode(kVK_Return), "return": CGKeyCode(kVK_Return),
    "tab": CGKeyCode(kVK_Tab),
    "escape": CGKeyCode(kVK_Escape), "esc": CGKeyCode(kVK_Escape),
    "backspace": CGKeyCode(kVK_Delete), "delete": CGKeyCode(kVK_Delete),
    "forward-delete": CGKeyCode(kVK_ForwardDelete),
    "space": CGKeyCode(kVK_Space),
    "left": CGKeyCode(kVK_LeftArrow), "right": CGKeyCode(kVK_RightArrow),
    "up": CGKeyCode(kVK_UpArrow), "down": CGKeyCode(kVK_DownArrow),
]

func cmdPressKey(_ args: Args) -> Never {
    let (pid, appElement, window) = resolveContext(args)
    let maxDepth = args.int("max-depth") ?? 40
    let selector = ElementSelector(args)
    let keyName = args.requireString("key")
    guard let keyCode = namedVirtualKeys[keyName.lowercased()] else {
        fail(
            "unknown --key \(keyName) (expected one of: \(namedVirtualKeys.keys.sorted().joined(separator: ", ")))"
        )
    }
    let modifierNames = (args.string("modifiers") ?? "")
        .split(separator: ",")
        .map { $0.trimmingCharacters(in: .whitespaces) }
        .filter { !$0.isEmpty }
    var flags: CGEventFlags = []
    for m in modifierNames {
        switch m {
        case "cmd": flags.insert(.maskCommand)
        case "shift": flags.insert(.maskShift)
        case "alt": flags.insert(.maskAlternate)
        case "ctrl": flags.insert(.maskControl)
        default: fail("unknown --modifiers entry \(m) (expected cmd/shift/alt/ctrl)")
        }
    }
    let focusVia = args.string("focus-via") ?? "none"
    let timeout = args.double("timeout") ?? 1.0

    if !selector.isEmpty {
        let element = resolveElement(in: window, selector: selector, maxDepth: maxDepth)
        switch focusVia {
        case "none": break
        case "ax-attribute":
            _ = AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue)
            _ = pollUntil(timeout: timeout) {
                axBool(element, kAXFocusedAttribute as String) ? true : nil
            }
        case "click":
            synthesizeClick(on: element)
            _ = pollUntil(timeout: timeout) {
                axBool(element, kAXFocusedAttribute as String) ? true : nil
            }
        default:
            fail("unknown --focus-via \(focusVia) (expected ax-attribute, click, or none)")
        }
    }

    func focusedElementJSON() -> [String: Any] {
        guard let raw = axCopyAttribute(appElement, kAXFocusedUIElementAttribute as String) else {
            return ["role": "", "title": "", "identifier": ""]
        }
        return AXElement(raw as! AXUIElement).jsonObject()
    }

    let before = focusedElementJSON()
    let src = CGEventSource(stateID: .hidSystemState)
    let down = CGEvent(keyboardEventSource: src, virtualKey: keyCode, keyDown: true)
    let up = CGEvent(keyboardEventSource: src, virtualKey: keyCode, keyDown: false)
    down?.flags = flags
    up?.flags = flags
    down?.post(tap: .cghidEventTap)
    up?.post(tap: .cghidEventTap)
    Thread.sleep(forTimeInterval: 0.05)
    let after = focusedElementJSON()

    emit(
        success: true,
        [
            "pid": Int(pid),
            "key": keyName,
            "modifiers": modifierNames,
            "focused_element_before": before,
            "focused_element_after": after,
        ])
}

// MARK: - wait-for

func cmdWaitFor(_ args: Args) -> Never {
    let (pid, _, window) = resolveContext(args)
    let maxDepth = args.int("max-depth") ?? 40
    let selector = ElementSelector(args)
    let condition = args.requireString("condition")
    let validConditions = ["exists", "not-exists", "enabled", "focused", "value-equals"]
    guard validConditions.contains(condition) else {
        fail(
            "unknown --condition \(condition) (expected one of: \(validConditions.joined(separator: ", ")))"
        )
    }
    let expectedValue = args.string("value")
    if condition == "value-equals" && expectedValue == nil {
        fail("--condition value-equals requires --value")
    }
    let timeout = args.double("timeout") ?? 5.0
    let interval = args.double("interval") ?? 0.1

    let start = Date()
    var lastMatchCount = 0
    let matched =
        pollUntil(timeout: timeout, interval: interval) { () -> Bool? in
            var nodeCount = 0
            let tree = buildAXTree(
                window, depth: 0, maxDepth: maxDepth, nodeCount: &nodeCount, maxNodes: 2000)
            let matches = filterNodes(tree.flattened(), matching: selector)
            lastMatchCount = matches.count
            switch condition {
            case "exists": return matches.count >= 1 ? true : nil
            case "not-exists": return matches.isEmpty ? true : nil
            case "enabled": return (matches.count == 1 && matches[0].axElement.enabled) ? true : nil
            case "focused": return (matches.count == 1 && matches[0].axElement.focused) ? true : nil
            case "value-equals":
                return (matches.count == 1
                    && axValueDescription(matches[0].axElement.value) == expectedValue) ? true : nil
            default: return nil  // unreachable — validated above
            }
        } == true
    let elapsed = Date().timeIntervalSince(start)

    emit(
        success: matched,
        [
            "pid": Int(pid),
            "condition": condition,
            "matched": matched,
            "timed_out": !matched,
            "elapsed_seconds": elapsed,
            "match_count": lastMatchCount,
        ])
}

// MARK: - entry point

let argv = Array(CommandLine.arguments.dropFirst())
guard let command = argv.first else {
    fail(
        "usage: macos-ui-driver <doctor|launch|terminate|list-windows|capture-window|focus-window|dump-tree|find|set-focus|click|point-click|drag|resize|type-text|press-key|wait-for> [options]"
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
case "dump-tree": cmdDumpTree(args)
case "find": cmdFind(args)
case "set-focus": cmdSetFocus(args)
case "click": cmdClick(args)
case "point-click": cmdPointClick(args)
case "drag": cmdDrag(args)
case "resize": cmdResize(args)
case "type-text": cmdTypeText(args)
case "press-key": cmdPressKey(args)
case "wait-for": cmdWaitFor(args)
default:
    fail("unknown command: \(command)")
}
