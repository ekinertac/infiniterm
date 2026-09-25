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
# WM_CHAR for text, WM_KEYDOWN/WM_KEYUP for Enter. Posting both was wrong
# (`dir` came out `ddiirr`, 2026-09-24) and so was the fix that followed it,
# key messages alone: the app's own pump calls TranslateMessage, which turns
# a posted WM_KEYDOWN into a WM_CHAR anyway, and whether that char lands on
# top of the keystroke depends on where the frame loop is. A quarter of the
# characters doubled, which is worse than all of them, because a short
# string usually came out right and the driver looked trustworthy
# (`abcdefghijklmnop` -> `abcdefghijjkllmnopp`, 2026-09-25). Only WM_CHAR,
# and the duplicate has nothing to duplicate from. Checked against
# chord.ps1, which sends real hardware input and types the same string
# cleanly.
#
# A character rather than a key also means the shifted ones work: a `:` used
# to arrive as `;`, because this took VkKeyScanW's virtual key and threw its
# shift bit away.
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

. "$PSScriptRoot\window.ps1"
$h = Get-IftWindow $ProcName

$WM_KEYDOWN = 0x0100
$WM_KEYUP = 0x0101
$WM_CHAR = 0x0102

# A shell reads faster than the app draws, and it paints on demand; without
# this pause between messages a whole line arrives inside one frame.
$GAP = 40

function Send-Char([char]$c) {
    [void][Post]::PostMessage($h, $WM_CHAR, [IntPtr][int]$c, [IntPtr]1)
    Start-Sleep -Milliseconds $GAP
}

function Send-Key([int]$vk) {
    # lParam: repeat count 1, the scan code in bits 16-23, and for the key-up
    # the transition and previous-state bits that say the key was held.
    $scan = [Post]::MapVirtualKeyW([uint32]$vk, 0)
    $down = [IntPtr](1 -bor ($scan -shl 16))
    $up = [IntPtr](1 -bor ($scan -shl 16) -bor 0xC0000000)
    [void][Post]::PostMessage($h, $WM_KEYDOWN, [IntPtr]$vk, $down)
    [void][Post]::PostMessage($h, $WM_KEYUP, [IntPtr]$vk, $up)
    Start-Sleep -Milliseconds $GAP
}

foreach ($c in $Text.ToCharArray()) { Send-Char $c }
# Enter is a key, not a character: a field acts on the keystroke.
if ($Enter) { Send-Key 0x0D }
"posted"
