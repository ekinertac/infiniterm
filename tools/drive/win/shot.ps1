# Screenshots one window by its process name, so the Windows port can be
# checked without asking Ekin to look at the screen every time.
#
# `PrintWindow` with PW_RENDERFULLCONTENT, not a read of the screen pixels:
# it asks the window to draw itself, so it needs neither focus nor a screen
# that is awake, and it never takes the foreground away from whatever Ekin
# is doing. The first version used `CopyFromScreen` and both of those bit
# on 2026-09-24: the box blanked at its ten-minute screen saver and every
# capture came back a white rectangle with "the handle is invalid" behind
# it. awake.ps1 stops the blanking; this stops depending on it.
#
# Related: run.ps1 (launch on a scratch data dir), type.ps1 and post.ps1
# (two ways to type into it), awake.ps1, docs/windows-handoff.md. The Mac's
# equivalent is tools/drive/lib.sh.
param(
    [string]$ProcName = 'infiniterm',
    [string]$Out = "$env:TEMP\ift-win-data\shot.png"
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Shot {
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

. "$PSScriptRoot\window.ps1"
$h = Get-IftWindow $ProcName

# DWMWA_EXTENDED_FRAME_BOUNDS (9): the real visible rect. GetWindowRect
# includes the invisible resize border and the capture comes out padded.
$r = New-Object Shot+RECT
[void][Shot]::DwmGetWindowAttribute($h, 9, [ref]$r, 16)
$w = $r.Right - $r.Left
$ht = $r.Bottom - $r.Top

New-Item -ItemType Directory -Force (Split-Path $Out) | Out-Null
$bmp = New-Object System.Drawing.Bitmap $w, $ht
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
# 2 = PW_RENDERFULLCONTENT, which is what gets a DirectX-composed window
# rather than an empty frame.
$ok = [Shot]::PrintWindow($h, $hdc, 2)
$g.ReleaseHdc($hdc)
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
if (-not $ok) { throw "PrintWindow refused $ProcName" }
"$Out ($w x $ht)"
