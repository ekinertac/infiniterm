# Types into one window by posting key messages straight to it, so it needs
# neither focus nor a screen that is awake, and Ekin's foreground window is
# never taken away.
#
# The other half of type.ps1, and both earn their place. type.ps1 sends REAL
# keystrokes through the OS, which is the only honest way to test the keymap
# (a chord, a layout, a dead key); this one fakes them at the window's own
# message queue, which is what to use when the question is about the app
# rather than about the keyboard.
#
# WM_KEYDOWN and WM_KEYUP only, no WM_CHAR: gpui turns each of those into a
# keystroke of its own, and posting both made every character arrive twice
# (`dir` came out `ddiirr`, 2026-09-24). Which also means this cannot type
# anything the key alone does not produce, an accented letter included; for
# those, and for anything the layout composes, use type.ps1.
#
# Related: run.ps1, shot.ps1, type.ps1, awake.ps1.
param(
    [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text,
    [switch]$Enter,
    [string]$ProcName = 'infiniterm'
)

$ErrorActionPreference = 'Stop'

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Post {
    [DllImport("user32.dll")] public static extern IntPtr PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern uint MapVirtualKeyW(uint code, uint type);
    [DllImport("user32.dll")] public static extern short VkKeyScanW(char c);
}
"@

$p = Get-Process $ProcName
$h = $p.MainWindowHandle
if ($h -eq 0) { throw "$ProcName has no window" }

$WM_KEYDOWN = 0x0100
$WM_KEYUP = 0x0101

function Send-Key([int]$vk) {
    # lParam: repeat count 1, the scan code in bits 16-23, and for the key-up
    # the transition and previous-state bits that say the key was held.
    $scan = [Post]::MapVirtualKeyW([uint32]$vk, 0)
    $down = [IntPtr](1 -bor ($scan -shl 16))
    $up = [IntPtr](1 -bor ($scan -shl 16) -bor 0xC0000000)
    [void][Post]::PostMessage($h, $WM_KEYDOWN, [IntPtr]$vk, $down)
    [void][Post]::PostMessage($h, $WM_KEYUP, [IntPtr]$vk, $up)
    # A shell reads faster than it draws; this is for the app's frame loop,
    # which paints on demand and would otherwise coalesce the whole line.
    Start-Sleep -Milliseconds 40
}

foreach ($c in $Text.ToCharArray()) {
    Send-Key ([Post]::VkKeyScanW($c) -band 0xFF)
}
if ($Enter) { Send-Key 0x0D }
"posted"
