# Sets up the environment a build with CEF needs, in the current PowerShell.
#
# Dot-source it, then run cargo:
#     . .\tools\drive\win\cef-env.ps1
#     cargo build -p infiniterm-ui
#
# Why it exists: cef-dll-sys's build script runs CMake with the Ninja
# generator on CEF's C++ wrapper, and that needs three things an interactive
# prompt has and a job started over ssh (or Win32_Process Create) does not:
#
#  1. CEF_PATH pointing at the Windows distribution. .cargo/config.toml sets
#     it to a macOS path with force = false, so the variable wins.
#  2. The compiler's environment (INCLUDE, LIB, PATH): what "x64 Native Tools"
#     sets up, taken from vcvars64.bat.
#  3. A ninja that can start. Scoop's shim goes through a `current` junction,
#     and a network logon may not follow one ("Shim: Could not create
#     process"); ~/.pwshrc fixes that for an interactive session only. So each
#     scoop app's real versioned directory goes on PATH and the shim directory
#     comes off it, because CMake finds ninja in the shim directory first.
#
# After a failed first attempt run `cargo clean -p cef-dll-sys`: CMake caches
# the ninja it found, shim path included.
#
# Related: docs/windows-handoff.md ("Setting up the box"), run.ps1.

$ErrorActionPreference = 'Stop'

$env:CEF_PATH = "$env:LOCALAPPDATA\cef"

$scoopApps = "$env:USERPROFILE\scoop\apps"
if (Test-Path $scoopApps) {
    Get-ChildItem $scoopApps -Directory | ForEach-Object {
        $version = Get-ChildItem $_.FullName -Directory |
            Where-Object Name -ne 'current' | Select-Object -First 1
        if ($version) { $env:PATH = "$($version.FullName);$env:PATH" }
    }
}

# Community first, then Build Tools: whichever Visual Studio is installed.
$vcvars = @(
    'C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat',
    'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat'
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $vcvars) { throw 'no vcvars64.bat: install the Visual Studio C++ workload' }
cmd /c "`"$vcvars`" >nul && set" | ForEach-Object {
    if ($_ -match '^([^=]+)=(.*)$') { Set-Item -Path "env:$($Matches[1])" -Value $Matches[2] }
}

$env:PATH = ($env:PATH -split ';' | Where-Object { $_ -notlike '*\scoop\shims*' }) -join ';'
