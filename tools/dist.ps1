# The Windows build that leaves this machine: an unpacked release folder, a
# zip of it, and latest-windows.json, the manifest an updater polls.
#
# The Mac's equivalent is tools/dist.sh, and this is deliberately smaller.
# There is no signing step: Authenticode needs a paid certificate and Ekin
# has not bought one, so SmartScreen will warn on a first run until a
# download has reputation. Nothing here pretends otherwise.
#
# A SEPARATE manifest from the Mac's `latest.json`, not a platform key
# inside it: an already-shipped Mac app polls that file and parses it with
# `update::parse_manifest`, and a shape it does not expect is a refusal on
# somebody else's machine. Same fields, different file.
#
# Refuses a dirty tree, like the Mac script and for the same reason: the
# build number is the commit count, and a build with uncommitted changes
# would carry a number that names a different tree.
#
# Layout inside the folder, which is also what `bundled_themes` and
# `hook_binary` look for beside the exe:
#
#     infiniterm.exe        the app
#     ift.exe               the command
#     infiniterm-hook.exe   what Claude Code's hooks run
#     themes/               520 schemes and their licence
#     LICENSE.txt
param(
    [string]$Out = "target/dist-win",
    # Where the updater would fetch from; only the manifest carries it.
    [string]$Base = "https://github.com/ekinertac/infiniterm-releases/releases/latest/download"
)

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

if (git status --porcelain --untracked-files=no) {
    throw "dist: the tree has uncommitted changes; commit first so the build number names this tree"
}

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$build = (git rev-list --count HEAD).Trim()
$commit = (git rev-parse --short HEAD).Trim()
$name = "infiniterm-$version-$build-x86_64"
$tag = "v$version-$build"

# Stamped into the binary: Windows has no bundle to read it back out of.
# `cargo clean -p` on the one crate that reads it, because `option_env!` is
# baked at compile time and cargo does not know the variable changed.
$env:INFINITERM_BUILD = $build
cargo clean -p infiniterm-ui
# The browser feature is off until CEF is built for Windows (phase 5 of
# docs/windows-handoff.md); `browser_stub.rs` draws those cards.
cargo build --release -p infiniterm-ui --no-default-features
cargo build --release -p infiniterm-cli
cargo build --release -p infiniterm-hook

$themes = "$env:USERPROFILE\Code\infiniterm-tauri\src-tauri\resources\themes"
if (-not (Test-Path $themes)) {
    throw "dist: no themes at $themes; the 520 schemes live in the archived Tauri app, see bundled_themes"
}

$folder = Join-Path $Out $name
Remove-Item -Recurse -Force $Out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $folder | Out-Null

foreach ($exe in 'infiniterm.exe', 'ift.exe', 'infiniterm-hook.exe') {
    Copy-Item "target/release/$exe" $folder
}
Copy-Item -Recurse $themes (Join-Path $folder 'themes')
Copy-Item LICENSE.txt $folder

$zip = Join-Path $Out "$name.zip"
Compress-Archive -Path $folder -DestinationPath $zip
$sha = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()

# The same fields `update::parse_manifest` reads, so a Windows updater needs
# no second parser.
@{
    version  = $version
    build    = [int]$build
    tag      = $tag
    pub_date = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    commit   = $commit
    url      = "$Base/$name.zip"
    sha256   = $sha
} | ConvertTo-Json | Set-Content (Join-Path $Out 'latest-windows.json')

Get-ChildItem $Out | Select-Object Name, Length | Format-Table -AutoSize
"dist: $tag ready in $Out"
"dist: unsigned, so SmartScreen warns until the download has reputation"
