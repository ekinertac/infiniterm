# Runs a script in the signed-in user's own desktop session and waits for it.
#
# Why it exists: a session started over ssh (or Win32_Process Create) has no
# desktop, so a window it starts is invisible to the screen and PrintWindow,
# EnumWindows and SendInput all see nothing. `run.ps1`, `shot.ps1`, `type.ps1`,
# `click.ps1` and the rest need a window station that has a screen. A
# scheduled task registered with /IT ("interactive only") runs as the
# logged-on user inside THEIR session, which has one. Verified 2026-10-22 on
# the gpu box: the task saw session 1 and a 7680x2160 virtual screen.
#
#     tools\drive\win\as-user.ps1 tools\drive\win\run.ps1
#     tools\drive\win\as-user.ps1 tools\drive\win\shot.ps1 -ScriptArgs '-Out','C:\Users\PC\a.png'
#     tools\drive\win\as-user.ps1 tools\drive\win\kill.ps1
#
# What it checks first, and refuses on: a console session that is Active (not
# Disc) and no LogonUI process (the lock screen). Nothing can be drawn for a
# locked or signed-out user; the Mac driver refuses a locked Mac the same way.
#
# What it costs the person at the box: the app's window appears on their
# screen while it runs, and the task's own console is hidden. It sends no
# input; `type.ps1` and `click.ps1` do, and they say so in their headers.
#
# The task is created, run, read and deleted every time, so nothing is left
# registered. Its output (the script's streams) comes back through a log file,
# because a task has no stdout; the exit code is the script's: 0, or 1 when it
# threw.
#
# Related: run.ps1, shot.ps1, kill.ps1, awake.ps1, docs/windows-handoff.md
# ("Looking at the screen").
param(
    [Parameter(Mandatory)][string]$Script,
    [string[]]$ScriptArgs = @(),
    [int]$TimeoutSec = 120
)

$ErrorActionPreference = 'Stop'

# 1: somebody is signed in at the console, and the screen is not locked.
$sessions = (query user 2>&1) | Out-String
if ($sessions -notmatch 'console\s+\d+\s+Active') {
    throw "no active console session, so there is no desktop to run in:`n$sessions"
}
if (Get-Process logonui -ErrorAction SilentlyContinue) {
    throw 'the screen is locked (LogonUI is running); unlock it first'
}

$full = (Resolve-Path $Script).Path
$id = [guid]::NewGuid().ToString('N').Substring(0, 8)
$task = "ift-as-user-$id"
$log = Join-Path $env:TEMP "$task.log"

# Single-quote every value so a path with a space or a `$` is one value. A
# token that is a parameter name (`-Out`) stays bare: quoted, it would be a
# positional value instead.
$quote = { param($s) "'" + ($s -replace "'", "''") + "'" }
$argText = ($ScriptArgs | ForEach-Object {
    if ($_ -match '^-[A-Za-z]\w*$') { $_ } else { & $quote $_ }
}) -join ' '
$body = "try { & $(& $quote $full) $argText *> $(& $quote $log); exit 0 } " +
        "catch { `$_ | Out-String | Add-Content $(& $quote $log); exit 1 }"
# A task's command line holds 261 characters, so the body goes in a small
# script of its own and the task runs that.
$wrapper = Join-Path $env:TEMP "$task.ps1"
Set-Content -Path $wrapper -Value $body
$run = "pwsh -NoProfile -WindowStyle Hidden -File `"$wrapper`""

try {
    schtasks /Create /TN $task /TR $run /SC ONCE /ST 00:00 /IT /RU $env:USERNAME /F 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not create the task" }
    schtasks /Run /TN $task 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not start the task" }

    # 267011 = has not run yet, 267009 = running; anything else is the exit code.
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    $result = $null
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 500
        $info = schtasks /Query /TN $task /V /FO LIST 2>&1 | Out-String
        $status = if ($info -match 'Status:\s+(\S+)') { $Matches[1] } else { '' }
        $last = if ($info -match 'Last Result:\s+(-?\d+)') { [int]$Matches[1] } else { 267011 }
        if ($status -ne 'Running' -and $last -ne 267011 -and $last -ne 267009) { $result = $last; break }
    }
    if ($null -eq $result) { throw "the script did not finish in $TimeoutSec s" }
    if (Test-Path $log) { Get-Content $log }
    exit $result
}
finally {
    schtasks /Delete /TN $task /F 2>&1 | Out-Null
    Remove-Item $log, $wrapper -ErrorAction SilentlyContinue
}
