# Stops every infiniterm process, and with -FreeLocks clears the lock on
# infiniterm.exe that a stuck one leaves behind so cargo can write it again.
#
# It ASKS FIRST, with WM_CLOSE, and only then kills. That is not politeness:
# a force-killed CEF app leaves a thread of the main process alive in the
# kernel, so the process stays enumerable with `HasExited` true and goes on
# holding `target\debug\libcef.dll`, and the next build dies on
# "Access is denied" from cef-dll-sys's build script. It stayed that way for
# the whole minute it was watched (2026-09-25), so it is not something to
# wait out. A normal quit runs `shutdown()` and the handles go. Killing is
# the fallback for a window that will not close, and is still needed for the
# subprocesses: Chromium's children are separate processes rather than a job
# the parent takes down, which is why this uses taskkill /T for the tree.
#
# `-FreeLocks` is the second, rarer job, and a switch rather than something
# this always does, because it costs a relink. It is for when the asking
# above did not work and the kill left the stuck thread behind: the
# terminated process goes on holding infiniterm.exe, and cargo's own
# "remove file" then fails with "Access is denied". Windows allows a RENAME
# of a locked image where it refuses a delete, which is the trick the
# installers use, so the stale exe goes aside under a name the next kill
# sweeps up. There is no way to ask whether that is needed without trying
# it: a binary that ran a moment ago is still memory-mapped, so every cheap
# test says locked and the first version of this script moved it aside on
# every call. Pass it when a build has actually failed, not before.
#
# IT DELIBERATELY LEAVES THE CEF DLLS ALONE, although the same process holds
# those too. Moving libcef.dll aside does not get a new one: cef-dll-sys's
# build script copies it, and cargo will not re-run a build script whose
# inputs have not changed, so the app then starts and dies instantly with no
# Chromium beside it (2026-09-25). Those locks only matter when that build
# script runs at all, which is a feature or version change rather than an
# ordinary edit. If one does block a build, the answer is `cargo clean -p
# cef-dll-sys` after this, and a minute of rebuilding.
#
# Related: run.ps1 (launch), window.ps1, docs/windows-handoff.md.
param(
    [switch]$FreeLocks,
    [string]$Exe = "$PSScriptRoot\..\..\..\target\debug\infiniterm.exe"
)

$ErrorActionPreference = 'Continue'

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Kill {
    [DllImport("user32.dll")] public static extern IntPtr PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
}
"@

# Ask. Only the app has a window, so this reaches the one process that owns
# the CEF shutdown; its children go with it.
foreach ($p in @(Get-Process infiniterm -ErrorAction SilentlyContinue)) {
    if ($p.MainWindowHandle -ne 0) {
        [void][Kill]::PostMessage($p.MainWindowHandle, 0x0010, [IntPtr]0, [IntPtr]0)  # WM_CLOSE
    }
}
for ($i = 0; $i -lt 40; $i++) {
    if (@(Get-Process infiniterm -ErrorAction SilentlyContinue).Count -eq 0) { break }
    Start-Sleep -Milliseconds 250
}

# Then insist, on whatever is left.
foreach ($p in @(Get-Process infiniterm, iftd -ErrorAction SilentlyContinue)) {
    & taskkill.exe /F /T /PID $p.Id 2>&1 | Out-Null
}
Start-Sleep -Milliseconds 800

$dir = Split-Path $Exe

# Sweep up what an earlier run put aside, now that nothing holds it.
Get-ChildItem $dir -Filter '*.stale-*' -ErrorAction SilentlyContinue |
    Remove-Item -Force -ErrorAction SilentlyContinue

if ($FreeLocks -and (Test-Path $Exe)) {
    try {
        [System.IO.File]::Open($Exe, 'Open', 'ReadWrite', 'None').Close()
    } catch {
        Move-Item $Exe "$Exe.stale-$(Get-Random)" -Force
        "moved a locked infiniterm.exe aside; rebuild before running"
    }
}

"killed"
