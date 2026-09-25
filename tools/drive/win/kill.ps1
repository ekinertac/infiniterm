# Stops every infiniterm process, and with -FreeExe clears a stale lock on
# target\debug\infiniterm.exe so the next `cargo build` can write it.
#
# `Stop-Process` on the app leaves its CEF subprocesses behind, because
# Chromium's children are separate processes rather than a job the parent
# takes down, so this uses taskkill /T for the tree.
#
# `-FreeExe` is the second, rarer job, and a switch rather than something
# this always does, because it costs a relink. A killed image stays
# enumerable while any handle to it is open: `Get-Process` still lists it
# with `HasExited` true, and cargo's own `remove file` then fails with
# "Access is denied" (2026-09-25). Windows allows a RENAME of a locked image
# where it refuses a delete, which is the trick the installers use, so the
# stale exe goes aside under a name the next kill sweeps up. There is no way
# to ask whether that is needed without trying it: an exe that ran a moment
# ago is still memory-mapped, so every cheap test says locked and the first
# version of this script moved the binary aside on every call. Pass it when
# a build has actually failed, not before.
#
# Related: run.ps1 (launch), window.ps1, docs/windows-handoff.md.
param(
    [switch]$FreeExe,
    [string]$Exe = "$PSScriptRoot\..\..\..\target\debug\infiniterm.exe"
)

$ErrorActionPreference = 'Continue'

foreach ($p in @(Get-Process infiniterm, iftd -ErrorAction SilentlyContinue)) {
    & taskkill.exe /F /T /PID $p.Id 2>&1 | Out-Null
}
Start-Sleep -Milliseconds 800

# Sweep up what an earlier run put aside, now that nothing holds it.
Get-ChildItem (Split-Path $Exe) -Filter 'infiniterm.stale-*.exe' -ErrorAction SilentlyContinue |
    Remove-Item -Force -ErrorAction SilentlyContinue

if ($FreeExe -and (Test-Path $Exe)) {
    $aside = Join-Path (Split-Path $Exe) ("infiniterm.stale-{0}.exe" -f (Get-Random))
    Move-Item $Exe $aside -Force
    "moved infiniterm.exe aside as $(Split-Path $aside -Leaf); rebuild before running"
}

"killed"
