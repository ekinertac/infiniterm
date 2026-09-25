# Clicks one point in the infiniterm window, in the coordinates shot.ps1
# writes its PNG in.
#
# That pairing is the whole design: a session looks at a screenshot, reads a
# pixel off it, and clicks it. Anything else means converting by hand between
# three origins, because a window has three. shot.ps1 captures the DWM
# extended frame bounds (the visible rectangle, without the invisible resize
# border), PostMessage wants client coordinates (inside the border, below the
# title bar), and the screen has its own. So this asks Windows for both rects
# and shifts by the difference.
#
# Posted messages, like post.ps1 and for the same reasons: the real pointer
# does not move, Ekin's foreground window is not taken, and a screen that has
# gone to sleep does not matter. The cost is the same one: this does not
# exercise the OS input path, so it cannot answer a question about the mouse
# itself, only about what the app does with a click.
#
# WM_MOUSEMOVE first, because a page tracks hover and a click with no move
# before it arrives somewhere the page was not expecting.
#
# Related: shot.ps1 (the coordinates), post.ps1 (typing), window.ps1.
param(
    [Parameter(Mandatory = $true)][int]$X,
    [Parameter(Mandatory = $true)][int]$Y,
    [ValidateSet('left', 'right')][string]$Button = 'left',
    [int]$Count = 1,
    [string]$ProcName = 'infiniterm'
)

$ErrorActionPreference = 'Stop'

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Click {
    [DllImport("user32.dll")] public static extern IntPtr PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
}
"@

. "$PSScriptRoot\window.ps1"
$h = Get-IftWindow $ProcName

# Where the client area's origin sits on screen, and where the captured
# rectangle's does; the difference is what a screenshot pixel needs.
$origin = New-Object Click+POINT
[void][Click]::ClientToScreen($h, [ref]$origin)
$frame = New-Object Click+RECT
[void][Click]::DwmGetWindowAttribute($h, 9, [ref]$frame, 16)

$cx = $X + $frame.Left - $origin.X
$cy = $Y + $frame.Top - $origin.Y
$l = [IntPtr](($cy -shl 16) -bor ($cx -band 0xFFFF))

$WM_MOUSEMOVE = 0x0200
$down = if ($Button -eq 'left') { 0x0201 } else { 0x0204 }
$up = if ($Button -eq 'left') { 0x0202 } else { 0x0205 }
$held = if ($Button -eq 'left') { 0x0001 } else { 0x0002 }

[void][Click]::PostMessage($h, $WM_MOUSEMOVE, [IntPtr]0, $l)
Start-Sleep -Milliseconds 60
for ($i = 0; $i -lt $Count; $i++) {
    [void][Click]::PostMessage($h, $down, [IntPtr]$held, $l)
    Start-Sleep -Milliseconds 40
    [void][Click]::PostMessage($h, $up, [IntPtr]0, $l)
    Start-Sleep -Milliseconds 60
}

"clicked $Button at client $cx,$cy"
