# Launches infiniterm on a scratch data dir and waits for its window.
#
# The Windows half of what `tools/drive/lib.sh` does on the Mac, and as small
# as it can be: there is no scenario runner here, because there is nothing on
# Windows that can assert what a window looks like. These three scripts exist
# so a session on the box can launch the app, type into it and look at a
# screenshot without asking Ekin to watch every time.
#
# NEVER runs on the real data dir. A second copy on the same one would find
# the endpoint held and exit (`hooks::listen`), but the save file is the
# thing worth not risking.
#
# Related: shot.ps1 (screenshot one window), type.ps1 (type into one window),
# docs/windows-handoff.md.
param(
    [string]$Data = "$env:TEMP\ift-win-data",
    [string]$Config = "$env:TEMP\ift-win-config",
    # The browser feature is off on Windows until CEF is built for it
    # (phase 5), so this is the binary --no-default-features produces.
    [string]$Exe = "$PSScriptRoot\..\..\..\target\debug\infiniterm.exe",
    [int]$WaitSeconds = 20
)

$ErrorActionPreference = 'Stop'

New-Item -ItemType Directory -Force $Data, $Config | Out-Null
$env:INFINITERM_DATA_DIR = $Data
$env:INFINITERM_CONFIG_DIR = $Config

if (-not (Test-Path $Exe)) {
    throw "no $Exe; cargo build -p infiniterm-ui --no-default-features"
}

# A force-killed instance's endpoint goes on answering for a second or two
# while Windows tears its handles down, and the app refuses to start while it
# does (see `transport::is_live` for why that is the safe way round). The
# driver kills instances constantly, so it waits that out rather than making
# every caller sleep.
$p = $null
for ($try = 1; $try -le 6; $try++) {
    $p = Start-Process -FilePath $Exe -PassThru `
        -RedirectStandardError "$Data\err.log" -RedirectStandardOutput "$Data\out.log"
    Start-Sleep -Milliseconds 700
    $p.Refresh()
    if (-not $p.HasExited) { break }
    $held = Select-String -Path "$Data\err.log" -Pattern 'holds the endpoint' -Quiet
    if (-not $held) { throw "infiniterm exited $($p.ExitCode); see $Data\err.log" }
    Start-Sleep -Seconds 1
}
if ($p.HasExited) { throw "the endpoint stayed held; another infiniterm is running" }

# PowerShell with a real profile takes seconds to draw its first prompt, and
# the window is up well before that; wait for the window, not the prompt.
$deadline = (Get-Date).AddSeconds($WaitSeconds)
while ((Get-Date) -lt $deadline) {
    $p.Refresh()
    if ($p.HasExited) { throw "infiniterm exited $($p.ExitCode); see $Data\err.log" }
    if ($p.MainWindowHandle -ne 0) { break }
    Start-Sleep -Milliseconds 300
}
if ($p.MainWindowHandle -eq 0) { throw "no window after $WaitSeconds s" }

"pid $($p.Id), data $Data"
