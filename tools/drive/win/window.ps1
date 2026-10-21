# The one infiniterm window handle, for every other script here.
#
# Dot-source it: `. "$PSScriptRoot\window.ps1"` then `$h = Get-IftWindow`.
#
# It exists because of CEF. On Windows Chromium re-executes the SAME exe for
# every subprocess it needs (renderer, GPU, utility, network), so once a
# browser card is open `Get-Process infiniterm` returns seven processes and
# `.MainWindowHandle` on that array is whichever one PowerShell put first,
# usually a subprocess with no window at all. Every driver script here used
# to do exactly that and every one of them broke the moment the browser
# feature went on.
#
# The app is the only one of the seven with a window, so that is the test;
# `--type=` on the command line is what tells Chromium's children apart and
# would work too, but it costs a WMI query per call.
#
# Related: run.ps1, shot.ps1, type.ps1, post.ps1, chord.ps1.
function Get-IftWindow {
    param([string]$ProcName = 'infiniterm')
    $procs = @(Get-Process $ProcName -ErrorAction SilentlyContinue)
    if ($procs.Count -eq 0) { throw "$ProcName is not running" }
    $win = @($procs | Where-Object { $_.MainWindowHandle -ne 0 })
    if ($win.Count -eq 0) { throw "$ProcName has no window" }
    if ($win.Count -gt 1) {
        throw "$($win.Count) $ProcName windows; kill the extra instance first"
    }
    $win[0].MainWindowHandle
}
