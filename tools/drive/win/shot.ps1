# Screenshots one window by its process name, so the Windows port can be
# checked without asking Ekin to look at the screen every time.
#
# Reads the pixels off the screen, so the window has to be in front and
# unobscured; that is why this raises and focuses it first. Asking the
# window to draw itself with PrintWindow would not need that, but gpui
# renders through DirectX into a swap chain and it was not tried here.
#
# Related: run.ps1 (launch on a scratch data dir), type.ps1 (type into it),
# docs/windows-handoff.md. The Mac's equivalent is tools/drive/lib.sh.
param(
    [string]$ProcName = 'infiniterm',
    [string]$Out = "$env:TEMP\ift-win-data\shot.png"
)

Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

$p = Get-Process $ProcName -ErrorAction Stop
$h = $p.MainWindowHandle
[void][Win]::ShowWindow($h, 9)          # SW_RESTORE
[void][Win]::SetForegroundWindow($h)
Start-Sleep -Milliseconds 700

# DWMWA_EXTENDED_FRAME_BOUNDS (9): the real visible rect, not the one with
# the invisible resize border GetWindowRect returns.
$r = New-Object Win+RECT
[void][Win]::DwmGetWindowAttribute($h, 9, [ref]$r, 16)
$w = $r.Right - $r.Left
$ht = $r.Bottom - $r.Top

$bmp = New-Object System.Drawing.Bitmap $w, $ht
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.Left, $r.Top, 0, 0, $bmp.Size)
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
"$Out ($w x $ht)"
