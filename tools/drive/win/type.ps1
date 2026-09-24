# Types into one window by process name, refusing if that window is not the
# one in front when the keys go out. Without that check a stray focus change
# sends the keystrokes into whatever Ekin happens to have open.
#
# Windows refuses SetForegroundWindow to a process that is not already in
# front; attaching our input queue to the current foreground thread is the
# standard way round it, and is undone straight after.
#
# Real keystrokes, not PostMessage: the point is to exercise the same path a
# person's keys take, through gpui's window proc and the keymap.
#
# Related: run.ps1, shot.ps1, docs/windows-handoff.md.
param(
    [Parameter(Mandatory = $true)][string]$Keys,
    [string]$ProcName = 'infiniterm'
)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Fg {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid);
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
}
"@

$p = Get-Process $ProcName -ErrorAction Stop
$h = $p.MainWindowHandle
[void][Fg]::ShowWindow($h, 9)

$me = [Fg]::GetCurrentThreadId()
$front = [Fg]::GetWindowThreadProcessId([Fg]::GetForegroundWindow(), [IntPtr]::Zero)
if ($front -ne $me) { [void][Fg]::AttachThreadInput($me, $front, $true) }
[void][Fg]::BringWindowToTop($h)
[void][Fg]::SetForegroundWindow($h)
if ($front -ne $me) { [void][Fg]::AttachThreadInput($me, $front, $false) }

Start-Sleep -Milliseconds 600
if ([Fg]::GetForegroundWindow() -ne $h) {
    throw "$ProcName is not in front; refusing to type"
}
$wsh = New-Object -ComObject WScript.Shell
$wsh.SendKeys($Keys)
Start-Sleep -Milliseconds 400
"sent"
