# infiniterm's PowerShell integration: the marks a card's shell writes so the app can colour the
# card (program_state.rs reads them off the output) and follow the directory.
#
#   OSC 133;D;<status>  before each prompt: the command that just ran ended with that status
#   OSC 7               before each prompt: the shell's directory, as a file address
#   OSC 133;C           when a line is accepted: a command started
#
# Loaded by the card's shell itself, AFTER the user's profile: shell_cmd.rs starts an interactive
# PowerShell with `-NoExit -Command "if ($env:INFINITERM_PWSH_INTEGRATION) { . ... }"`, and the app
# sets that variable only for a local shell card (shell_integration.rs, `pwsh_env`). Loading after
# the profile is what lets this wrap whatever prompt the profile installed (oh-my-posh, starship,
# the default) instead of being replaced by it. The zsh twin is infiniterm.zsh.
#
# What it must not do: change what the user's prompt sees. The original prompt runs with `$?` and
# `$LASTEXITCODE` as the last command left them, the way VS Code's integration does it.

if ($global:__ift_marks_loaded) { return }
$global:__ift_marks_loaded = $true

$global:__ift_original_prompt = $function:prompt

function global:prompt {
    $ok = $global:?
    $code = $global:LASTEXITCODE
    # A native command's own status when it failed; 1 for a cmdlet or a script that failed.
    $status = 0
    if (-not $ok) { $status = if ($code) { $code } else { 1 } }
    $esc = [char]27
    $bel = [char]7
    $marks = "$esc]133;D;$status$bel"
    try {
        if ($PWD.Provider.Name -eq 'FileSystem') {
            $marks += "$esc]7;" + ([System.Uri]::new($PWD.ProviderPath)).AbsoluteUri + $bel
        }
    } catch { }
    # The original prompt must see the status the command left, not ours.
    $global:LASTEXITCODE = $code
    if (-not $ok) { Write-Error 'infiniterm' -ErrorAction Ignore }
    $text = & $global:__ift_original_prompt
    $marks + $text
}

# `C` goes out when PSReadLine hands back the accepted line, before the command runs.
if (Get-Module -Name PSReadLine) {
    $global:__ift_original_readline = $function:PSConsoleHostReadLine
    function global:PSConsoleHostReadLine {
        $line = & $global:__ift_original_readline
        [Console]::Write("$([char]27)]133;C$([char]7)")
        $line
    }
}
