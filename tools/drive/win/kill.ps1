# Stops every infiniterm process and frees target\debug\infiniterm.exe so the
# next `cargo build` can write it.
#
# Both halves earn their place. `Stop-Process` on the app leaves its CEF
# subprocesses behind, because Chromium's children are separate processes
# rather than a job the parent takes down, so this uses taskkill /T for the
# tree. And a killed image stays enumerable while any handle to it is open:
# `Get-Process` still lists it with `HasExited` true, and cargo's `remove
# file` fails with "Access is denied" (2026-09-25). Windows allows a RENAME
# of a locked image where it refuses a delete, which is the trick the
# installers use, so the stale exe goes aside under a name the next kill
# sweeps up. That path is only for a lock that outlives the process; the
# normal case renames nothing.
#
# Related: run.ps1 (launch), window.ps1, docs/windows-handoff.md.
param(
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

if (Test-Path $Exe) {
    try {
        # The cheapest test for a lock that does not destroy the file: open
        # it for writing. cargo's own failure is a delete, and a delete we
        # could not undo.
        [System.IO.File]::Open($Exe, 'Open', 'ReadWrite', 'None').Close()
    } catch {
        $aside = Join-Path (Split-Path $Exe) ("infiniterm.stale-{0}.exe" -f (Get-Random))
        Move-Item $Exe $aside -Force
        "moved a locked infiniterm.exe aside as $(Split-Path $aside -Leaf)"
    }
}

"killed"
