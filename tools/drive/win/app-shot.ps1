# One command, from an ssh session: launch infiniterm on a scratch data dir in
# the signed-in user's desktop, wait for it to settle, save a PNG of its
# window, and stop it.
#
# It is `run.ps1`, `shot.ps1` and `kill.ps1` in a row, each through
# `as-user.ps1` (an ssh session has no desktop; that script runs them in the
# user's own session). Use it when the question is "what does it look like".
# For a longer look, or to type and click first, pass -Keep and drive the app
# with `as-user.ps1` and `post.ps1` / `click.ps1`, then `kill.ps1` yourself.
#
#     pwsh -NoProfile -File tools\drive\win\app-shot.ps1 -Out C:\Users\PC\a.png
#     scp win-gpu-box:C:/Users/PC/a.png .            # on the Mac
#
# REFUSES to start if any `infiniterm` process is already running, and says
# so. `kill.ps1` stops every process of that name, and `shot.ps1` takes the
# one window it finds, so with the person's own infiniterm open the first
# would end their session and the second could photograph their canvas.
# Nothing here ever touches a running instance: stop it yourself or do not
# drive.
#
# -Exe: which build. The default is the CEF build in target\debug, which as of
# 2026-10-22 CRASHES inside libcef.dll (0x80000003) about a second after
# start when launched this way, on the September build as well as today's, so
# it is not a regression in master; launched from the desktop by hand it was
# fine in September. A build with `--no-default-features` (and
# `--target-dir target-nocef` to leave the CEF exe alone) runs and captures
# fine. Until that is understood, pass -Exe for the browser-less build.
#
# What it costs the person at the box: the window opens on their screen for
# about ten seconds. No input is sent.
#
# Related: as-user.ps1, run.ps1, shot.ps1, kill.ps1, docs/windows-handoff.md
# ("Looking at the screen from ssh").
param(
    [string]$Exe = "$PSScriptRoot\..\..\..\target\debug\infiniterm.exe",
    [string]$Data = "$env:TEMP\ift-win-data",
    [string]$Out = "$env:TEMP\ift-win-data\shot.png",
    [int]$SettleSeconds = 4,
    [switch]$Keep
)

$ErrorActionPreference = 'Stop'

$running = @(Get-Process infiniterm -ErrorAction SilentlyContinue)
if ($running.Count -gt 0) {
    throw "an infiniterm is already running (pid $($running[0].Id)); not touching it. Close it, or drive on another machine."
}

$as = Join-Path $PSScriptRoot 'as-user.ps1'
$exe = (Resolve-Path $Exe).Path

& $as (Join-Path $PSScriptRoot 'run.ps1') -ScriptArgs '-Exe', $exe, '-Data', $Data
if ($LASTEXITCODE -ne 0) { throw "run.ps1 failed ($LASTEXITCODE)" }

try {
    Start-Sleep -Seconds $SettleSeconds
    & $as (Join-Path $PSScriptRoot 'shot.ps1') -ScriptArgs '-Out', $Out
    if ($LASTEXITCODE -ne 0) {
        throw "shot.ps1 failed ($LASTEXITCODE): the app is gone or has no window; see the Application event log for a crash"
    }
}
finally {
    if (-not $Keep) { & $as (Join-Path $PSScriptRoot 'kill.ps1') | Out-Null }
}
