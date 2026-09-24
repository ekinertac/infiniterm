# Keeps the display awake for as long as this process lives, so a driven run
# can be watched and (with the older CopyFromScreen path) screenshotted.
#
# This box blanks at ten minutes (a screen saver, `scrnsave.scr`, not secure,
# so it does not lock) and turns the display off at fifteen. Both are idle
# timers, and both stopped a screen capture dead on 2026-09-24.
#
# Changes NOTHING on the system. `SetThreadExecutionState` is per thread and
# per process: the request goes away the moment this process does, so
# stopping it is `Stop-Process`, not undoing a setting. The zero-pixel mouse
# nudge is there because the execution state suppresses display-off and
# sleep but not every screen saver; moving the pointer by nothing resets the
# idle timer that one reads. A keystroke would do it too, and would land in
# whatever window has focus, which is Ekin's.
#
# Related: shot.ps1 captures with PrintWindow and does not actually need
# this; run.ps1, type.ps1. The Mac's equivalent question is answered by
# tools/locked.swift, which refuses to drive a locked Mac rather than
# keeping it awake.
param(
    [switch]$Stop,
    [string]$PidFile = "$env:TEMP\ift-awake.pid",
    [int]$EverySeconds = 60
)

if ($Stop) {
    if (Test-Path $PidFile) {
        $id = Get-Content $PidFile
        Stop-Process -Id $id -ErrorAction SilentlyContinue
        Remove-Item $PidFile
        "stopped $id"
    } else {
        "not running"
    }
    return
}

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Awake {
    [DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint flags);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, uint d, IntPtr x);
}
"@

# ES_CONTINUOUS makes the request stand until it is changed;
# ES_DISPLAY_REQUIRED keeps the screen on, ES_SYSTEM_REQUIRED keeps the
# machine out of sleep. The `u` suffixes matter: without them PowerShell
# reads 0x80000000 as a signed Int32 and the cast overflows.
$ES_CONTINUOUS = 0x80000000u
$ES_SYSTEM_REQUIRED = 0x00000001u
$ES_DISPLAY_REQUIRED = 0x00000002u
$FLAGS = $ES_CONTINUOUS -bor $ES_SYSTEM_REQUIRED -bor $ES_DISPLAY_REQUIRED
$MOUSEEVENTF_MOVE = 0x0001

$PID | Set-Content $PidFile
while ($true) {
    [void][Awake]::SetThreadExecutionState($FLAGS)
    [Awake]::mouse_event($MOUSEEVENTF_MOVE, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Seconds $EverySeconds
}
