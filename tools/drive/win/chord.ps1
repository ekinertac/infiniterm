# Presses one chord as real hardware would, with a chosen SIDE of Ctrl.
#
# This is the tool the Windows keymap needs and the other two cannot be.
# `type.ps1` goes through SendKeys, which sends a virtual key with scan code
# 0 and cannot say which Ctrl it meant; `post.ps1` posts messages straight to
# the window, which never touches the keyboard STATE that decides a chord
# (`keycode::roles` reads GetKeyState for the two Ctrls). Only SendInput
# carrying both the virtual key and the scan code exercises the real path.
#
# Takes a VK and a scan code rather than a character on purpose: the whole
# point of the Windows keymap work is that the chord comes from the physical
# key, so the test has to name a physical key. Scan 0x14 is T wherever T is
# printed; on Turkish Q, scan 0x1A is the key marked ğ and US keyboards
# print [ on.
#
# Related: run.ps1, shot.ps1, post.ps1, type.ps1, infiniterm-ui/src/keycode.rs.
param(
    # 'left' is where Cmd sits on a Mac-order keyboard, 'right' is Caps Lock.
    [ValidateSet('left', 'right', 'none')][string]$Ctrl = 'none',
    [switch]$Shift,
    [switch]$Alt,
    [Parameter(Mandatory = $true)][int]$Vk,
    [Parameter(Mandatory = $true)][int]$Scan,
    [string]$ProcName = 'infiniterm'
)

$ErrorActionPreference = 'Stop'

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Chord {
    [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT {
        public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo;
    }
    [StructLayout(LayoutKind.Sequential)] public struct INPUT {
        public uint type; public KEYBDINPUT ki; public int pad1; public int pad2;
    }
    [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] p, int size);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid);
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
}
"@

$EXTENDED = 0x0001
$KEYUP = 0x0002

function New-Key([int]$vk, [int]$scan, [bool]$extended, [bool]$up) {
    $i = New-Object Chord+INPUT
    $i.type = 1
    $k = New-Object Chord+KEYBDINPUT
    $k.wVk = [uint16]$vk
    $k.wScan = [uint16]$scan
    $f = 0
    if ($extended) { $f = $f -bor $EXTENDED }
    if ($up) { $f = $f -bor $KEYUP }
    $k.dwFlags = [uint32]$f
    $i.ki = $k
    $i
}

# Windows refuses SetForegroundWindow to a process that is not already in
# front; attaching our input queue to the current foreground thread is the
# way round it. It does not always take on the first try, and a chord sent
# to the wrong window lands in whatever Ekin has open, so this insists.
$h = (Get-Process $ProcName).MainWindowHandle
if ($h -eq 0) { throw "$ProcName has no window" }
for ($try = 1; $try -le 8; $try++) {
    [void][Chord]::ShowWindow($h, 9)
    $me = [Chord]::GetCurrentThreadId()
    $front = [Chord]::GetWindowThreadProcessId([Chord]::GetForegroundWindow(), [IntPtr]::Zero)
    if ($front -ne $me) { [void][Chord]::AttachThreadInput($me, $front, $true) }
    [void][Chord]::BringWindowToTop($h)
    [void][Chord]::SetForegroundWindow($h)
    if ($front -ne $me) { [void][Chord]::AttachThreadInput($me, $front, $false) }
    Start-Sleep -Milliseconds 400
    if ([Chord]::GetForegroundWindow() -eq $h) { break }
}
if ([Chord]::GetForegroundWindow() -ne $h) {
    throw "$ProcName would not come to the front; refusing to send"
}

# VK_LCONTROL 0xA2 and VK_RCONTROL 0xA3 share scan 0x1D, the right one
# extended; VK_LSHIFT 0xA0 is scan 0x2A and VK_LMENU 0xA4 is scan 0x38.
$seq = @()
if ($Ctrl -eq 'left') { $seq += (New-Key 0xA2 0x1D $false $false) }
if ($Ctrl -eq 'right') { $seq += (New-Key 0xA3 0x1D $true $false) }
if ($Shift) { $seq += (New-Key 0xA0 0x2A $false $false) }
if ($Alt) { $seq += (New-Key 0xA4 0x38 $false $false) }
$seq += (New-Key $Vk $Scan $false $false)
$seq += (New-Key $Vk $Scan $false $true)
if ($Alt) { $seq += (New-Key 0xA4 0x38 $false $true) }
if ($Shift) { $seq += (New-Key 0xA0 0x2A $false $true) }
if ($Ctrl -eq 'left') { $seq += (New-Key 0xA2 0x1D $false $true) }
if ($Ctrl -eq 'right') { $seq += (New-Key 0xA3 0x1D $true $true) }

$size = [System.Runtime.InteropServices.Marshal]::SizeOf([type]([Chord+INPUT]))
$sent = [Chord]::SendInput([uint32]$seq.Count, [Chord+INPUT[]]$seq, $size)
if ($sent -ne $seq.Count) { throw "SendInput took $sent of $($seq.Count)" }
"ctrl=$Ctrl shift=$Shift alt=$Alt vk=0x$('{0:X2}' -f $Vk) scan=0x$('{0:X2}' -f $Scan)"
