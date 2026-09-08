<#
.SYNOPSIS
    Repository-owned Windows native E2E adapter. Stable command surface over the external
    Microsoft `winapp` CLI (`winapp ui ...`) plus a minimal Win32 helper for process/window
    lifecycle, foreground verification, bounds/DPI/monitor metadata, and move/resize.

.DESCRIPTION
    See docs/design/tools/windows_ui_driver_design.md for the architecture this implements and
    docs/agents/winui3-e2e.md for the operational tester procedure. tools/windows-ui-driver/README.md
    documents the command surface for humans.

    This script never performs Win32 input injection (SendInput/mouse_event/keybd_event/PostMessage
    as a click or keystroke substitute) and never vendors or auto-installs `winapp`. All UIA
    inspection/actions, real mouse/keyboard input, and screenshot capture are delegated to `winapp
    ui`. This script owns only: stable command names/JSON shape, process/window lifecycle, HWND
    enumeration, foreground request+verification, bounds/DPI/monitor metadata, native move/resize,
    and normalization of `winapp` results/failures into one fixed error taxonomy.

.NOTES
    Every command prints exactly one JSON object to stdout and sets the process exit code (0 success,
    1 failure). stderr is reserved for this script's own diagnostics and never mixed into the stdout
    JSON. Set ELWINDUI_WINAPP_PATH to override the external backend executable/script (test-only;
    real E2E must not set it).
#>

param(
    [Parameter(Position = 0, Mandatory = $true)]
    [ValidateSet(
        'doctor', 'launch', 'list-windows', 'focus-window',
        'inspect', 'search', 'invoke', 'get-value', 'get-property', 'set-focus', 'wait-for',
        'capture-window', 'point-click', 'drag', 'send-keys',
        'move-window', 'resize-window', 'terminate'
    )]
    [string]$Command,

    [Parameter(Position = 1, ValueFromRemainingArguments = $true)]
    [string[]]$RestArgs
)

$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------------------
# Win32 interop (process/window lifecycle only -- no input injection here).
# ---------------------------------------------------------------------------

if (-not ("ElwindUI.Win32Driver" -as [type])) {
    Add-Type -Namespace ElwindUI -Name Win32Driver -MemberDefinition @"
[DllImport("user32.dll")]
public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

// Enumerates entirely in compiled code -- the EnumWindows native callback is a C# lambda here,
// never a PowerShell scriptblock marshaled as a delegate. Calling *into* native code (ordinary
// P/Invoke) is safe; native code calling *back into* a PowerShell scriptblock mid-enumeration was
// suspected of occasional unreliability and is avoided entirely by keeping the whole callback loop
// in .NET.
public static System.Collections.Generic.List<IntPtr> EnumWindowsForProcess(int targetPid) {
    var results = new System.Collections.Generic.List<IntPtr>();
    EnumWindows(delegate(IntPtr hWnd, IntPtr lParam) {
        int pid;
        GetWindowThreadProcessId(hWnd, out pid);
        if (pid == targetPid) {
            results.Add(hWnd);
        }
        return true;
    }, IntPtr.Zero);
    return results;
}

[DllImport("user32.dll")]
public static extern int GetWindowThreadProcessId(IntPtr hWnd, out int lpdwProcessId);

[DllImport("user32.dll")]
[return: MarshalAs(UnmanagedType.Bool)]
public static extern bool IsWindowVisible(IntPtr hWnd);

[DllImport("user32.dll")]
[return: MarshalAs(UnmanagedType.Bool)]
public static extern bool IsWindowEnabled(IntPtr hWnd);

[DllImport("user32.dll")]
[return: MarshalAs(UnmanagedType.Bool)]
public static extern bool IsIconic(IntPtr hWnd);

[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern int GetWindowTextW(IntPtr hWnd, System.Text.StringBuilder lpString, int nMaxCount);

[DllImport("user32.dll")]
[return: MarshalAs(UnmanagedType.Bool)]
public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);

[DllImport("user32.dll")]
public static extern IntPtr GetForegroundWindow();

[DllImport("user32.dll")]
[return: MarshalAs(UnmanagedType.Bool)]
public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);

[DllImport("user32.dll")]
[return: MarshalAs(UnmanagedType.Bool)]
public static extern bool SetForegroundWindow(IntPtr hWnd);

[DllImport("user32.dll")]
[return: MarshalAs(UnmanagedType.Bool)]
public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);

[DllImport("user32.dll")]
public static extern uint GetDpiForWindow(IntPtr hWnd);

[DllImport("user32.dll")]
public static extern IntPtr MonitorFromWindow(IntPtr hWnd, uint dwFlags);

[DllImport("user32.dll", CharSet = CharSet.Unicode)]
[return: MarshalAs(UnmanagedType.Bool)]
public static extern bool GetMonitorInfoW(IntPtr hMonitor, ref MONITORINFOEX lpmi);

// doctor-only environment probe -- not used for any input injection.
[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern IntPtr OpenInputDesktop(uint dwFlags, bool fInherit, uint dwDesiredAccess);

[DllImport("user32.dll")]
[return: MarshalAs(UnmanagedType.Bool)]
public static extern bool CloseDesktop(IntPtr hDesktop);

// Windows' CreateProcess inherits *every* inheritable handle this process currently holds into
// any child started with handle inheritance enabled (which redirecting a child's stdio requires),
// not just the three handles explicitly passed as that child's own stdin/stdout/stderr. When this
// script's own stdout/stderr/stdin were themselves inherited from *its* parent (e.g. a caller that
// redirected this script's output to read it), a child this script launches with its own
// redirection would otherwise also inherit those same handles as an unrelated side effect --
// keeping a long-lived child's duplicate write-end of the parent's own stdout pipe open forever,
// so the parent's own caller never sees EOF even after this script has already exited. Clearing
// the inherit flag on this process's own standard handles at startup (once) prevents that.
[DllImport("kernel32.dll", SetLastError = true)]
public static extern IntPtr GetStdHandle(int nStdHandle);

[DllImport("kernel32.dll", SetLastError = true)]
[return: MarshalAs(UnmanagedType.Bool)]
public static extern bool SetHandleInformation(IntPtr hObject, uint dwMask, uint dwFlags);

public const int STD_INPUT_HANDLE = -10;
public const int STD_OUTPUT_HANDLE = -11;
public const int STD_ERROR_HANDLE = -12;
public const uint HANDLE_FLAG_INHERIT = 0x00000001;

public static void MakeOwnStdHandlesNonInheritable() {
    foreach (int which in new[] { STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE }) {
        IntPtr h = GetStdHandle(which);
        if (h != IntPtr.Zero && h != new IntPtr(-1)) {
            SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0);
        }
    }
}

[StructLayout(LayoutKind.Sequential)]
public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }

[StructLayout(LayoutKind.Sequential)]
public struct RECTM { public int Left; public int Top; public int Right; public int Bottom; }

[StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
public struct MONITORINFOEX {
    public uint cbSize;
    public RECTM rcMonitor;
    public RECTM rcWork;
    public uint dwFlags;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)]
    public string szDevice;
}
"@
}

# See MakeOwnStdHandlesNonInheritable's own comment: must run once, early, before this script
# might launch any child with its own redirection.
[ElwindUI.Win32Driver]::MakeOwnStdHandlesNonInheritable()

$SWP_NOZORDER = 0x0004
$SWP_NOACTIVATE = 0x0010
$SW_RESTORE = 9
$MONITOR_DEFAULTTONEAREST = 2

# ---------------------------------------------------------------------------
# JSON envelope helpers. Defined before argument parsing (below) because
# ConvertTo-ArgMap itself is invoked immediately, at script top level, and
# must be able to fail closed (Emit-UsageError) on malformed input rather
# than silently accepting it -- a function's own *definition* may reference
# another function defined later in the script, but a top-level *call* may
# not, since PowerShell does not hoist top-level function definitions ahead
# of top-level statement execution.
# ---------------------------------------------------------------------------

function Emit-Result {
    param([hashtable]$Obj)
    $json = $Obj | ConvertTo-Json -Depth 16 -Compress
    Write-Output $json
    if ($Obj.ContainsKey('success') -and $Obj['success']) { exit 0 } else { exit 1 }
}

function Emit-UsageError {
    param([string]$Message)
    Emit-Result @{ success = $false; category = 'usage_error'; error = $Message }
}

# ---------------------------------------------------------------------------
# Argument parsing: manual "--key value" / "--flag" parser (semantics are
# fixed by the owning Issue's contract; exact flag spelling is a repository
# convention, not a winapp passthrough).
# ---------------------------------------------------------------------------

function ConvertTo-ArgMap {
    param([string[]]$Tokens)
    $map = @{}
    $i = 0
    while ($i -lt $Tokens.Count) {
        $tok = $Tokens[$i]
        if ($tok -like '--*') {
            $name = $tok.Substring(2)
            $next = if ($i + 1 -lt $Tokens.Count) { $Tokens[$i + 1] } else { $null }
            if ($name -eq 'arg') {
                # `--arg <value>` is launch's repeated application-argument syntax, mirroring the
                # AppKit driver's own repeated `--arg <a>` form (tools/macos-ui-driver). Unlike
                # every other flag, the immediately following token is always consumed verbatim as
                # this occurrence's value -- including one that itself begins with "--" -- so a
                # launched application argument such as --some-app-option is never misread as a
                # driver flag. Every occurrence accumulates, in order, into a list so repeated
                # `--arg one --arg two` is never collapsed to only the last value (the generic
                # single-value-per-key map below cannot represent that). A trailing `--arg` with
                # no following token at all is malformed input, not a silently-dropped no-op.
                if ($null -eq $next) {
                    Emit-UsageError '--arg requires a value'
                }
                if (-not $map.ContainsKey($name)) { $map[$name] = New-Object System.Collections.Generic.List[string] }
                $map[$name].Add($next)
                $i += 2
                continue
            }
            if ($null -ne $next -and $next -notlike '--*') {
                $map[$name] = $next
                $i += 2
            }
            else {
                $map[$name] = $true
                $i += 1
            }
        }
        else {
            $i += 1
        }
    }
    return $map
}

$Args2 = ConvertTo-ArgMap -Tokens $RestArgs

function Get-Arg {
    param([string]$Name, $Default = $null)
    if ($Args2.ContainsKey($Name)) { return $Args2[$Name] }
    return $Default
}

function Require-Arg {
    param([string]$Name)
    if (-not $Args2.ContainsKey($Name)) {
        Emit-UsageError "missing required argument --$Name"
    }
    $value = $Args2[$Name]
    if ($value -is [bool]) {
        # Every Require-Arg caller wants a string/numeric value, never a bare boolean flag. This
        # single check catches both a genuinely missing value (--path with nothing after it) and
        # the following-token-looked-like-a-flag case (--path --wait-window-timeout 10, where the
        # generic parser above could not tell that --wait-window-timeout was never meant as
        # --path's own value) -- both must fail closed as usage_error, never silently launch a
        # process whose path is the literal string "True".
        Emit-UsageError "--$Name requires a value"
    }
    return $value
}

# ---------------------------------------------------------------------------
# External backend resolver + invocation wrapper.
# ---------------------------------------------------------------------------

function Get-WinAppPath {
    if ($env:ELWINDUI_WINAPP_PATH) { return $env:ELWINDUI_WINAPP_PATH }
    return 'winapp'
}

# Known winapp failure-category tokens (see docs/design/tools/windows_ui_driver_design.md and
# the winapp-cli UI Automation reference). Order matters: more specific tokens are checked first.
$script:EnvironmentBlockerTokens = @('no_interactive_desktop', 'foreground_not_target', 'access_denied', 'AccessDenied', 'access is denied')
$script:TargetErrorTokens = @('target_moved', 'no_target', 'element_not_found', 'ambiguous_selector', 'No running app found', 'No UIA window found', 'Multiple windows match')
$script:UsageErrorTokens = @('invalid_arguments')

function Get-BackendErrorCategory {
    param([string]$CombinedText)
    foreach ($t in $script:EnvironmentBlockerTokens) {
        if ($CombinedText -like "*$t*") { return 'environment_blocker' }
    }
    foreach ($t in $script:TargetErrorTokens) {
        if ($CombinedText -like "*$t*") { return 'target_error' }
    }
    foreach ($t in $script:UsageErrorTokens) {
        if ($CombinedText -like "*$t*") { return 'usage_error' }
    }
    # Unrecognized backend failure: do not silently claim it is a product failure or a specific
    # environment/tool condition we did not actually detect. Default to target_error so an E2E
    # case still investigates the specific action/target rather than assuming a host-wide block.
    return 'target_error'
}

function Invoke-WinApp {
    param([string[]]$BackendArgs)
    $backend = Get-WinAppPath
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    if ($backend -like '*.ps1') {
        # Test-only path (ELWINDUI_WINAPP_PATH pointed at tests/fake-winapp.ps1): a .ps1 file has
        # no direct Windows file association safe to rely on, so route it through pwsh explicitly.
        $psi.FileName = (Get-Process -Id $PID).Path
        $psi.ArgumentList.Add('-NoProfile')
        $psi.ArgumentList.Add('-File')
        $psi.ArgumentList.Add($backend)
    }
    else {
        $psi.FileName = $backend
    }
    foreach ($a in $BackendArgs) { $psi.ArgumentList.Add($a) }
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardInput = $true
    $psi.UseShellExecute = $false
    try {
        $proc = [System.Diagnostics.Process]::Start($psi)
        $proc.StandardInput.Close()
    }
    catch {
        return @{
            Launched = $false
            ExitCode = -1
            StdOut   = ''
            StdErr   = $_.Exception.Message
            Json     = $null
        }
    }
    $stdout = $proc.StandardOutput.ReadToEnd()
    $stderr = $proc.StandardError.ReadToEnd()
    $proc.WaitForExit()
    $parsed = $null
    if ($stdout -and $stdout.Trim().StartsWith('{')) {
        try { $parsed = $stdout | ConvertFrom-Json -ErrorAction Stop } catch { $parsed = $null }
    }
    return @{
        Launched = $true
        ExitCode = $proc.ExitCode
        StdOut   = $stdout
        StdErr   = $stderr
        Json     = $parsed
    }
}

function Test-WinAppMissing {
    param($InvokeResult)
    if (-not $InvokeResult.Launched) { return $true }
    return $false
}

# ---------------------------------------------------------------------------
# Window discovery (driver-owned; deterministic HWND metadata).
# ---------------------------------------------------------------------------

function Get-WindowInfo {
    param([IntPtr]$Hwnd)
    $rect = New-Object ElwindUI.Win32Driver+RECT
    [void][ElwindUI.Win32Driver]::GetWindowRect($Hwnd, [ref]$rect)
    $sb = New-Object System.Text.StringBuilder 512
    [void][ElwindUI.Win32Driver]::GetWindowTextW($Hwnd, $sb, 512)
    $pidOut = 0
    [void][ElwindUI.Win32Driver]::GetWindowThreadProcessId($Hwnd, [ref]$pidOut)
    $dpi = 96
    try { $dpi = [ElwindUI.Win32Driver]::GetDpiForWindow($Hwnd) } catch { $dpi = 96 }
    $monitor = $null
    $monitorBounds = $null
    $workArea = $null
    $monitorHandle = [ElwindUI.Win32Driver]::MonitorFromWindow($Hwnd, $MONITOR_DEFAULTTONEAREST)
    if ($monitorHandle -ne [IntPtr]::Zero) {
        $mi = New-Object ElwindUI.Win32Driver+MONITORINFOEX
        $mi.cbSize = [System.Runtime.InteropServices.Marshal]::SizeOf([type][ElwindUI.Win32Driver+MONITORINFOEX])
        if ([ElwindUI.Win32Driver]::GetMonitorInfoW($monitorHandle, [ref]$mi)) {
            $monitor = $mi.szDevice
            $monitorBounds = @{ left = $mi.rcMonitor.Left; top = $mi.rcMonitor.Top; right = $mi.rcMonitor.Right; bottom = $mi.rcMonitor.Bottom }
            $workArea = @{ left = $mi.rcWork.Left; top = $mi.rcWork.Top; right = $mi.rcWork.Right; bottom = $mi.rcWork.Bottom }
        }
    }
    return @{
        hwnd           = ('0x{0:X}' -f [int64]$Hwnd)
        hwnd_decimal   = [int64]$Hwnd
        pid            = $pidOut
        title          = $sb.ToString()
        visible        = [ElwindUI.Win32Driver]::IsWindowVisible($Hwnd)
        enabled        = [ElwindUI.Win32Driver]::IsWindowEnabled($Hwnd)
        left           = $rect.Left
        top            = $rect.Top
        width          = ($rect.Right - $rect.Left)
        height         = ($rect.Bottom - $rect.Top)
        dpi            = $dpi
        monitor        = $monitor
        monitor_bounds = $monitorBounds
        work_area      = $workArea
    }
}

function Get-WindowsForPid {
    param([int]$TargetPid, [bool]$ShowHidden = $false)
    $hwnds = [ElwindUI.Win32Driver]::EnumWindowsForProcess($TargetPid)
    $results = New-Object System.Collections.Generic.List[object]
    foreach ($h in $hwnds) {
        $info = Get-WindowInfo -Hwnd $h
        if ($ShowHidden -or ($info.visible -and ($info.width -gt 0) -and ($info.height -gt 0))) {
            $results.Add($info)
        }
    }
    # The unary comma prevents PowerShell's pipeline output from flattening a 0- or 1-element
    # List[object] back into a bare scalar/hashtable, which would otherwise make a caller's
    # `.Count` silently count the single window's own hashtable keys instead of the window count.
    return , $results
}

function ConvertTo-Hwnd {
    param($HwndArg)
    if ($null -eq $HwndArg) { return [IntPtr]::Zero }
    $s = [string]$HwndArg
    if ($s.StartsWith('0x') -or $s.StartsWith('0X')) {
        return [IntPtr]([Convert]::ToInt64($s.Substring(2), 16))
    }
    return [IntPtr]([Convert]::ToInt64($s, 10))
}

function Get-TargetArgs {
    param($TargetPid, $Hwnd)
    if ($Hwnd) {
        $h = ConvertTo-Hwnd $Hwnd
        return @('-w', ([int64]$h).ToString())
    }
    if ($TargetPid) { return @('-a', [string]$TargetPid) }
    Emit-UsageError 'one of --pid or --hwnd is required'
}

# ---------------------------------------------------------------------------
# Commands
# ---------------------------------------------------------------------------

function Cmd-Doctor {
    $winappPath = Get-WinAppPath
    $result = Invoke-WinApp -BackendArgs @('--version')
    # winapp prints a one-time welcome/telemetry banner before the version on a machine's very
    # first invocation ever; the version itself is always the last non-empty stdout line.
    $versionLines = @($result.StdOut -split "`r?`n" | Where-Object { $_.Trim() -ne '' })
    $winappVersion = if ($versionLines.Count -gt 0) { $versionLines[-1].Trim() } else { $result.StdOut.Trim() }
    $sessionId = 0
    try { $sessionId = [System.Diagnostics.Process]::GetCurrentProcess().SessionId } catch { $sessionId = -1 }
    $fgHwnd = [ElwindUI.Win32Driver]::GetForegroundWindow()
    $inputDesktopProbe = 'unknown'
    try {
        $DESKTOP_READOBJECTS = 0x0001
        $h = [ElwindUI.Win32Driver]::OpenInputDesktop(0, $false, $DESKTOP_READOBJECTS)
        if ($h -ne [IntPtr]::Zero) {
            $inputDesktopProbe = 'available'
            [void][ElwindUI.Win32Driver]::CloseDesktop($h)
        }
        else {
            $inputDesktopProbe = 'unavailable'
        }
    }
    catch {
        $inputDesktopProbe = 'unknown'
    }

    if (Test-WinAppMissing $result) {
        Emit-Result @{
            success              = $false
            category             = 'tool_error'
            platform             = 'windows'
            winapp_available     = $false
            install_command      = 'winget install Microsoft.winappcli --source winget'
            error                = "winapp was not found on PATH (or ELWINDUI_WINAPP_PATH is invalid): $winappPath"
            session_id           = $sessionId
            foreground_hwnd      = ('0x{0:X}' -f [int64]$fgHwnd)
            input_desktop_probe  = $inputDesktopProbe
            notes                = @('winapp is an external dependency; this driver never auto-installs it.')
        }
    }

    # A successfully *started* `winapp --version` process is not the same as a *healthy* one --
    # a broken install can still launch and then exit non-zero, or print nothing. Both are
    # tool_error, not success:true; doctor must never claim a healthy backend from process launch
    # alone.
    if (($result.ExitCode -ne 0) -or (-not $winappVersion)) {
        Emit-Result @{
            success              = $false
            category             = 'tool_error'
            platform             = 'windows'
            winapp_available     = $false
            install_command      = 'winget install Microsoft.winappcli --source winget'
            error                = "winapp --version exited with code $($result.ExitCode) or produced no version output"
            backend_exit_code    = $result.ExitCode
            backend_stdout       = $result.StdOut
            backend_stderr       = $result.StdErr
            session_id           = $sessionId
            foreground_hwnd      = ('0x{0:X}' -f [int64]$fgHwnd)
            input_desktop_probe  = $inputDesktopProbe
            notes                = @('winapp launched but did not report a healthy version -- treat this the same as a missing/broken install.')
        }
    }

    Emit-Result @{
        success              = $true
        platform             = 'windows'
        winapp_available     = $true
        winapp_version       = $winappVersion
        uia_probe_available  = $true
        session_id           = $sessionId
        foreground_hwnd      = ('0x{0:X}' -f [int64]$fgHwnd)
        input_desktop_probe  = $inputDesktopProbe
        notes                = @('real_mouse_input_available is not reported here; it is proven only by a live case whose application postcondition changed.')
    }
}

function Cmd-Launch {
    $path = Require-Arg 'path'
    $cwd = Get-Arg 'cwd'
    $timeout = [double](Get-Arg 'wait-window-timeout' 0)
    # Each `--arg <value>` occurrence is collected in order by ConvertTo-ArgMap (see its own
    # comment); read it back as a plain array here regardless of whether zero, one, or many
    # occurrences were given.
    $launchArgs = @()
    if ($Args2.ContainsKey('arg')) { $launchArgs = @($Args2['arg']) }
    # This driver never connects the launched product process's own stdin/stdout/stderr to a
    # driver-owned pipe. Two invariants together make that safe for a caller capturing *this
    # script's own* stdout (as an E2E case driving this script through a nested pwsh invocation
    # does):
    #   1. MakeOwnStdHandlesNonInheritable() (called once, at script startup, before any child is
    #      launched) clears the inherit flag on this process's own standard handles, so the
    #      launched (long-lived) child cannot inherit this script's own stdout pipe and hold its
    #      write end open past this script's own exit -- that inherited-handle propagation was the
    #      original cause of a caller's read never reaching EOF even though this script itself had
    #      already exited.
    #   2. Simply not setting RedirectStandardOutput/Error/Input here means there is no
    #      driver-owned pipe for the child to fill in the first place -- a redirected-but-unread
    #      pipe backpressures the child once the OS buffer fills, which is a *different* deadlock
    #      class than (1) and was rejected as a design (see docs/design/tools/windows_ui_driver_design.md).
    #      A launched process's stdout/stderr is therefore not part of this driver's JSON protocol
    #      and is never captured -- application log capture is out of scope for `launch`.
    # CreateNoWindow additionally suppresses the console-host window Windows would otherwise
    # allocate for this workspace's examples (default console subsystem, no
    # `#![windows_subsystem = "windows"]`), which was separately observed to appear as an extra,
    # nearly-full-screen top-level window for the same PID and get misidentified as the main window.
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $path
    foreach ($a in $launchArgs) { $psi.ArgumentList.Add($a) }
    if ($cwd) { $psi.WorkingDirectory = $cwd }
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    try {
        $proc = [System.Diagnostics.Process]::Start($psi)
    }
    catch {
        Emit-Result @{ success = $false; category = 'target_error'; error = "failed to start process: $($_.Exception.Message)" }
    }
    $result = @{ success = $true; pid = $proc.Id; process_path = $path }
    if ($timeout -gt 0) {
        # Poll a cheap, plain .NET Process property -- not Get-WindowsForPid's EnumWindows
        # P/Invoke callback -- so the hot polling loop never repeatedly marshals a fresh PowerShell
        # scriptblock delegate into unmanaged code. That full enumeration (for complete metadata:
        # DPI, monitor, every candidate window) runs exactly once, after a main window exists.
        $deadline = [DateTime]::UtcNow.AddSeconds($timeout)
        $windows = @()
        while ([DateTime]::UtcNow -lt $deadline) {
            $proc.Refresh()
            if ($proc.MainWindowHandle -ne [IntPtr]::Zero) { break }
            Start-Sleep -Milliseconds 100
        }
        if ($proc.MainWindowHandle -ne [IntPtr]::Zero) {
            $windows = Get-WindowsForPid -TargetPid $proc.Id
        }
        if ($windows.Count -eq 1) {
            $result['window'] = $windows[0]
        }
        elseif ($windows.Count -gt 1) {
            $result['windows'] = $windows
            $result['notes'] = @('multiple candidate windows; caller must select by title/geometry')
        }
        else {
            $result['success'] = $false
            $result['category'] = 'target_error'
            $result['error'] = 'no window appeared for the launched process within the timeout'
        }
    }
    Emit-Result $result
}

function Cmd-ListWindows {
    $pid_ = [int](Require-Arg 'pid')
    $showHidden = [bool](Get-Arg 'show-hidden' $false)
    $windows = Get-WindowsForPid -TargetPid $pid_ -ShowHidden $showHidden
    Emit-Result @{ success = $true; pid = $pid_; windows = $windows }
}

function Cmd-FocusWindow {
    $hwndArg = Require-Arg 'hwnd'
    $timeout = [double](Get-Arg 'timeout' 3)
    $hwnd = ConvertTo-Hwnd $hwndArg
    $before = [ElwindUI.Win32Driver]::GetForegroundWindow()
    if ([ElwindUI.Win32Driver]::IsIconic($hwnd)) {
        [void][ElwindUI.Win32Driver]::ShowWindowAsync($hwnd, $SW_RESTORE)
    }
    [void][ElwindUI.Win32Driver]::SetForegroundWindow($hwnd)
    $deadline = [DateTime]::UtcNow.AddSeconds($timeout)
    $after = [ElwindUI.Win32Driver]::GetForegroundWindow()
    while (($after -ne $hwnd) -and ([DateTime]::UtcNow -lt $deadline)) {
        Start-Sleep -Milliseconds 50
        [void][ElwindUI.Win32Driver]::SetForegroundWindow($hwnd)
        $after = [ElwindUI.Win32Driver]::GetForegroundWindow()
    }
    $success = ($after -eq $hwnd)
    $result = @{
        success          = $success
        hwnd             = $hwndArg
        foreground_before = ('0x{0:X}' -f [int64]$before)
        foreground_after  = ('0x{0:X}' -f [int64]$after)
    }
    if (-not $success) {
        $result['category'] = 'environment_blocker'
        $result['error'] = 'target window did not become the actual foreground window within the timeout'
    }
    Emit-Result $result
}

function Invoke-UiaCommand {
    param([string[]]$BaseArgs, [bool]$RequireSuccessTrue = $true)
    $backendArgs = $BaseArgs + @('--json')
    $r = Invoke-WinApp -BackendArgs $backendArgs
    if (Test-WinAppMissing $r) {
        Emit-Result @{
            success          = $false
            category         = 'tool_error'
            error            = 'winapp was not found on PATH (or ELWINDUI_WINAPP_PATH is invalid)'
            install_command  = 'winget install Microsoft.winappcli --source winget'
        }
    }
    $combined = "$($r.StdOut)`n$($r.StdErr)"
    $success = ($r.ExitCode -eq 0)
    $result = @{
        success           = $success
        backend           = $r.Json
        backend_exit_code = $r.ExitCode
    }
    # Only carry the raw streams when they add information beyond the already-parsed `backend`
    # JSON -- an actionable failure, or a stdout that didn't parse as JSON at all.
    if ((-not $success) -or ($null -eq $r.Json)) {
        $result['backend_stdout'] = $r.StdOut
        $result['backend_stderr'] = $r.StdErr
    }
    if (-not $success) {
        $result['category'] = Get-BackendErrorCategory -CombinedText $combined
    }
    Emit-Result $result
}

function Cmd-Inspect {
    $target = Get-TargetArgs -TargetPid (Get-Arg 'pid') -Hwnd (Get-Arg 'hwnd')
    $selector = Get-Arg 'selector'
    $baseArgs = @('ui', 'inspect')
    if ($selector) { $baseArgs += $selector }
    $baseArgs += $target
    if (Get-Arg 'depth') { $baseArgs += @('--depth', (Get-Arg 'depth')) }
    if (Get-Arg 'interactive') { $baseArgs += '--interactive' }
    Invoke-UiaCommand -BaseArgs $baseArgs
}

function Cmd-Search {
    $target = Get-TargetArgs -TargetPid (Get-Arg 'pid') -Hwnd (Get-Arg 'hwnd')
    $query = Require-Arg 'query'
    $baseArgs = @('ui', 'search', $query) + $target
    if (Get-Arg 'max') { $baseArgs += @('--max', (Get-Arg 'max')) }
    Invoke-UiaCommand -BaseArgs $baseArgs
}

function Cmd-Invoke {
    $target = Get-TargetArgs -TargetPid (Get-Arg 'pid') -Hwnd (Get-Arg 'hwnd')
    $selector = Require-Arg 'selector'
    Invoke-UiaCommand -BaseArgs (@('ui', 'invoke', $selector) + $target)
}

function Cmd-GetValue {
    $target = Get-TargetArgs -TargetPid (Get-Arg 'pid') -Hwnd (Get-Arg 'hwnd')
    $selector = Require-Arg 'selector'
    Invoke-UiaCommand -BaseArgs (@('ui', 'get-value', $selector) + $target)
}

function Cmd-GetProperty {
    $target = Get-TargetArgs -TargetPid (Get-Arg 'pid') -Hwnd (Get-Arg 'hwnd')
    $selector = Require-Arg 'selector'
    $baseArgs = @('ui', 'get-property', $selector) + $target
    if (Get-Arg 'property') { $baseArgs += @('-p', (Get-Arg 'property')) }
    Invoke-UiaCommand -BaseArgs $baseArgs
}

function Cmd-SetFocus {
    $target = Get-TargetArgs -TargetPid (Get-Arg 'pid') -Hwnd (Get-Arg 'hwnd')
    $selector = Require-Arg 'selector'
    Invoke-UiaCommand -BaseArgs (@('ui', 'focus', $selector) + $target)
}

function Cmd-WaitFor {
    $target = Get-TargetArgs -TargetPid (Get-Arg 'pid') -Hwnd (Get-Arg 'hwnd')
    $selector = Get-Arg 'selector'
    $baseArgs = @('ui', 'wait-for')
    if ($selector) { $baseArgs += $selector }
    $baseArgs += $target
    if (Get-Arg 'value') { $baseArgs += @('--value', (Get-Arg 'value')) }
    if (Get-Arg 'property') { $baseArgs += @('--property', (Get-Arg 'property')) }
    if (Get-Arg 'contains') { $baseArgs += '--contains' }
    if (Get-Arg 'gone') { $baseArgs += '--gone' }
    $timeoutMs = Get-Arg 'timeout-ms' 5000
    $baseArgs += @('--timeout', $timeoutMs)
    Invoke-UiaCommand -BaseArgs $baseArgs
}

function Cmd-CaptureWindow {
    $target = Get-TargetArgs -TargetPid (Get-Arg 'pid') -Hwnd (Get-Arg 'hwnd')
    $out = Require-Arg 'output'
    $captureScreen = [bool](Get-Arg 'capture-screen' $false)
    $focus = [bool](Get-Arg 'focus' $false)
    $baseArgs = @('ui', 'screenshot') + $target + @('--output', $out)
    if ($captureScreen) { $baseArgs += '--capture-screen' }
    elseif ($focus) { $baseArgs += '--focus' }
    $r = Invoke-WinApp -BackendArgs ($baseArgs + '--json')
    if (Test-WinAppMissing $r) {
        Emit-Result @{ success = $false; category = 'tool_error'; error = 'winapp was not found'; install_command = 'winget install Microsoft.winappcli --source winget' }
    }
    $success = ($r.ExitCode -eq 0)
    $mode = if ($captureScreen) { 'screen' } else { 'window' }
    $result = @{
        success           = $success
        path              = $out
        capture_mode      = $mode
        backend           = $r.Json
        backend_exit_code = $r.ExitCode
        backend_stdout    = $r.StdOut
        backend_stderr    = $r.StdErr
    }
    if ($success -and (Test-Path $out)) {
        $result['file_exists'] = $true
        $result['file_size']   = (Get-Item $out).Length
    }
    elseif ($success) {
        $result['file_exists'] = $false
    }
    if (-not $success) {
        $result['category'] = Get-BackendErrorCategory -CombinedText "$($r.StdOut)`n$($r.StdErr)"
    }
    Emit-Result $result
}

function Cmd-PointClick {
    $target = Get-TargetArgs -TargetPid (Get-Arg 'pid') -Hwnd (Get-Arg 'hwnd')
    $x = Require-Arg 'x'
    $y = Require-Arg 'y'
    $button = Get-Arg 'button' 'left'
    # winapp has no raw-coordinate click verb; a zero-distance real-mouse drag at the same point
    # is the documented-equivalent primitive (see design doc Section 4 / contract Section 2.11).
    $baseArgs = @('ui', 'drag', "$x,$y", "$x,$y") + $target
    if ($button -eq 'right') { $baseArgs += '--right' }
    $r = Invoke-WinApp -BackendArgs ($baseArgs + '--json')
    if (Test-WinAppMissing $r) {
        Emit-Result @{ success = $false; category = 'tool_error'; error = 'winapp was not found'; install_command = 'winget install Microsoft.winappcli --source winget' }
    }
    $success = ($r.ExitCode -eq 0)
    $result = @{
        success           = $success
        point             = @{ x = [int]$x; y = [int]$y }
        button            = $button
        backend           = $r.Json
        backend_exit_code = $r.ExitCode
        backend_stdout    = $r.StdOut
        backend_stderr    = $r.StdErr
    }
    if (-not $success) {
        $result['category'] = Get-BackendErrorCategory -CombinedText "$($r.StdOut)`n$($r.StdErr)"
    }
    Emit-Result $result
}

function Cmd-Drag {
    $target = Get-TargetArgs -TargetPid (Get-Arg 'pid') -Hwnd (Get-Arg 'hwnd')
    $from = if (Get-Arg 'from-selector') { Get-Arg 'from-selector' } else { "$(Require-Arg 'from-x'),$(Require-Arg 'from-y')" }
    $to = if (Get-Arg 'to-selector') { Get-Arg 'to-selector' } else { "$(Require-Arg 'to-x'),$(Require-Arg 'to-y')" }
    $button = Get-Arg 'button' 'left'
    $baseArgs = @('ui', 'drag', $from, $to) + $target
    if ($button -eq 'right') { $baseArgs += '--right' }
    if (Get-Arg 'hold-ms') { $baseArgs += @('--hold-ms', (Get-Arg 'hold-ms')) }
    if (Get-Arg 'dwell-ms') { $baseArgs += @('--dwell-ms', (Get-Arg 'dwell-ms')) }
    Invoke-UiaCommand -BaseArgs $baseArgs
}

function Cmd-SendKeys {
    $target = Get-TargetArgs -TargetPid (Get-Arg 'pid') -Hwnd (Get-Arg 'hwnd')
    $keys = Require-Arg 'keys'
    $via = Get-Arg 'via' 'send-input'
    $baseArgs = @('ui', 'send-keys', $keys) + $target + @('--via', $via)
    if (Get-Arg 'target') { $baseArgs += @('--target', (Get-Arg 'target')) }
    if (Get-Arg 'verbatim') { $baseArgs += '--verbatim' }
    if (Get-Arg 'allow-system-keys') { $baseArgs += '--allow-system-keys' }
    Invoke-UiaCommand -BaseArgs $baseArgs
}

function Cmd-MoveWindow {
    $hwnd = ConvertTo-Hwnd (Require-Arg 'hwnd')
    $left = [int](Require-Arg 'left')
    $top = [int](Require-Arg 'top')
    $before = Get-WindowInfo -Hwnd $hwnd
    [void][ElwindUI.Win32Driver]::SetWindowPos($hwnd, [IntPtr]::Zero, $left, $top, 0, 0, ($SWP_NOZORDER -bor $SWP_NOACTIVATE -bor 0x0001))
    $after = Get-WindowInfo -Hwnd $hwnd
    Emit-Result @{ success = $true; before = $before; after = $after }
}

function Cmd-ResizeWindow {
    $hwnd = ConvertTo-Hwnd (Require-Arg 'hwnd')
    $width = [int](Require-Arg 'width')
    $height = [int](Require-Arg 'height')
    $before = Get-WindowInfo -Hwnd $hwnd
    [void][ElwindUI.Win32Driver]::SetWindowPos($hwnd, [IntPtr]::Zero, 0, 0, $width, $height, ($SWP_NOZORDER -bor $SWP_NOACTIVATE -bor 0x0002))
    $after = Get-WindowInfo -Hwnd $hwnd
    Emit-Result @{ success = $true; before = $before; after = $after }
}

function Cmd-Terminate {
    $pid_ = [int](Require-Arg 'pid')
    $timeout = [double](Get-Arg 'timeout' 5)
    try {
        $proc = Get-Process -Id $pid_ -ErrorAction Stop
    }
    catch {
        Emit-Result @{ success = $true; pid = $pid_; terminated = $true; forced = $false; notes = @('process was already gone') }
    }
    $forced = $false
    $graceful = $proc.CloseMainWindow()
    if ($graceful) {
        $exited = $proc.WaitForExit([int]($timeout * 1000))
        if (-not $exited) {
            $forced = $true
            Stop-Process -Id $pid_ -Force -ErrorAction SilentlyContinue
        }
    }
    else {
        $forced = $true
        Stop-Process -Id $pid_ -Force -ErrorAction SilentlyContinue
    }
    Start-Sleep -Milliseconds 200
    $stillAlive = $true
    try { Get-Process -Id $pid_ -ErrorAction Stop | Out-Null } catch { $stillAlive = $false }
    Emit-Result @{ success = (-not $stillAlive); pid = $pid_; terminated = (-not $stillAlive); forced = $forced }
}

# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------

switch ($Command) {
    'doctor' { Cmd-Doctor }
    'launch' { Cmd-Launch }
    'list-windows' { Cmd-ListWindows }
    'focus-window' { Cmd-FocusWindow }
    'inspect' { Cmd-Inspect }
    'search' { Cmd-Search }
    'invoke' { Cmd-Invoke }
    'get-value' { Cmd-GetValue }
    'get-property' { Cmd-GetProperty }
    'set-focus' { Cmd-SetFocus }
    'wait-for' { Cmd-WaitFor }
    'capture-window' { Cmd-CaptureWindow }
    'point-click' { Cmd-PointClick }
    'drag' { Cmd-Drag }
    'send-keys' { Cmd-SendKeys }
    'move-window' { Cmd-MoveWindow }
    'resize-window' { Cmd-ResizeWindow }
    'terminate' { Cmd-Terminate }
    default { Emit-UsageError "unknown command: $Command" }
}
