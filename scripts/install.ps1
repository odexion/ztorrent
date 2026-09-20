# Install ztorrent on Windows from the latest GitHub release.
#
#   powershell -ExecutionPolicy Bypass -c "irm https://raw.githubusercontent.com/odexion/ztorrent/main/scripts/install.ps1 | iex"
#
# The installer runs in currentUser mode: it installs under %LOCALAPPDATA%, adds
# a Start menu entry and an uninstaller, and never asks for administrator rights.
#
# -ExecutionPolicy Bypass applies to this one process, so that a script piped in
# from the web runs at all. It is PowerShell's guard against running scripts by
# accident, not a Windows security control. Nothing here turns off SmartScreen or
# Defender, and nothing excludes ztorrent from them -- Defender scans the
# download as it always would. What this does avoid is the "Windows protected
# your PC" prompt that a browser download earns by tagging the file with the mark
# of the web. In its place the download is checked against the SHA-256 GitHub
# publishes for the asset, which clicking through that prompt never does.
#
# Environment:
#   ZTORRENT_VERSION  tag to install, e.g. v0.5.3   (default: the latest release)
#   ZTORRENT_REPO     owner/name to install from    (default: odexion/ztorrent)
#   GITHUB_TOKEN      optional, only to lift the anonymous API rate limit
#   NO_COLOR          set to anything to turn off colour

$ErrorActionPreference = 'Stop'

$Repo = if ($env:ZTORRENT_REPO) { $env:ZTORRENT_REPO } else { 'odexion/ztorrent' }

# ------------------------------------------------------------------ terminal

$Color = (-not $env:NO_COLOR)

# Redrawing over a line only makes sense on a console. Piped into a file or a CI
# log, a bare carriage return is just a stray byte mid-transcript.
$Interactive = $true
try { $Interactive = -not [Console]::IsOutputRedirected } catch {}

function Write-Step { param([string]$Text)
    if ($Color) { Write-Host '  * ' -ForegroundColor Cyan -NoNewline } else { Write-Host '  * ' -NoNewline }
    Write-Host $Text
}

function Write-Ok { param([string]$Text)
    if ($Color) { Write-Host '  + ' -ForegroundColor Green -NoNewline } else { Write-Host '  + ' -NoNewline }
    Write-Host $Text
}

function Write-Note { param([string]$Text)
    if ($Color) { Write-Host "  $Text" -ForegroundColor DarkGray } else { Write-Host "  $Text" }
}

function Stop-WithError { param([string]$Text)
    if ($Color) { Write-Host '  x ' -ForegroundColor Red -NoNewline } else { Write-Host '  x ' -NoNewline }
    Write-Host $Text
    exit 1
}

function Format-Size { param([double]$Bytes)
    $units = @('B', 'KB', 'MB', 'GB')
    $i = 0
    while ($Bytes -ge 1024 -and $i -lt 3) { $Bytes = $Bytes / 1024; $i++ }
    if ($i -eq 0) { return ('{0:N0} {1}' -f $Bytes, $units[$i]) }
    return ('{0:N1} {1}' -f $Bytes, $units[$i])
}

# A single status line, redrawn in place.
function Write-Line { param([string]$Text)
    $padded = ("`r" + $Text).PadRight(72)
    Write-Host $padded -NoNewline
}

function Clear-Line {
    Write-Host ("`r" + (' ' * 72) + "`r") -NoNewline
}

Write-Host ''

if ($PSVersionTable.PSVersion.Major -lt 5) {
    Stop-WithError "this needs PowerShell 5.1 or newer (found $($PSVersionTable.PSVersion))"
}

# PowerShell 5.1 still defaults to TLS 1.0 on older builds; github.com refuses it.
try {
    [Net.ServicePointManager]::SecurityProtocol =
        [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch {}

# ---------------------------------------------------------------- platform

# PROCESSOR_ARCHITECTURE reports the architecture of *this process*, so a 32-bit
# PowerShell on a 64-bit machine says x86 and leaves the real answer in
# PROCESSOR_ARCHITEW6432. Ask the runtime first, where it can answer.
function Get-Arch {
    try {
        $os = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
        if ($os -eq 'Arm64') { return 'arm64' }
        if ($os -eq 'X64')   { return 'x64' }
    } catch {}

    $raw = $env:PROCESSOR_ARCHITEW6432
    if (-not $raw) { $raw = $env:PROCESSOR_ARCHITECTURE }
    if ($raw -eq 'ARM64') { return 'arm64' }
    if ($raw -eq 'AMD64') { return 'x64' }
    Stop-WithError "unsupported architecture: $raw (x64 and arm64 only)"
}

# ---------------------------------------------------------------- release

function Invoke-GitHub { param([string]$Uri)
    $headers = @{
        'User-Agent' = 'ztorrent-install'
        'Accept'     = 'application/vnd.github+json'
    }
    if ($env:GITHUB_TOKEN) { $headers['Authorization'] = "Bearer $env:GITHUB_TOKEN" }
    return Invoke-RestMethod -Uri $Uri -Headers $headers -UseBasicParsing
}

# ---------------------------------------------------------------- download

# Invoke-WebRequest buffers the whole body before writing it and draws a progress
# bar that cannot be styled, so the stream is copied by hand and the line
# redrawn as the bytes land. No Authorization header goes out with this one:
# the asset URL redirects to a CDN on another host, and the token has no
# business following it there.
function Save-Asset { param([string]$Uri, [string]$Path, [long]$Total)
    $request = [System.Net.HttpWebRequest]::Create($Uri)
    $request.UserAgent = 'ztorrent-install'
    $request.AllowAutoRedirect = $true

    $response = $request.GetResponse()
    if ($Total -le 0) { $Total = $response.ContentLength }

    $in = $response.GetResponseStream()
    $out = [System.IO.File]::Create($Path)
    $buffer = New-Object byte[] 131072
    $got = 0L
    $started = Get-Date
    $lastDraw = [DateTime]::MinValue

    if (-not $Interactive) { Write-Step ('downloading {0}' -f (Format-Size $Total)) }

    try {
        while (($read = $in.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $out.Write($buffer, 0, $read)
            $got += $read

            if (-not $Interactive) { continue }

            # A redraw every frame costs more than the download does on a fast link.
            if (((Get-Date) - $lastDraw).TotalMilliseconds -lt 80) { continue }
            $lastDraw = Get-Date

            $seconds = ((Get-Date) - $started).TotalSeconds
            $rate = 0
            if ($seconds -gt 0) { $rate = $got / $seconds }

            if ($Total -gt 0) {
                $percent = [int](100 * $got / $Total)
                $text = '{0,3}%  {1} / {2}  {3}/s' -f $percent, (Format-Size $got), (Format-Size $Total), (Format-Size $rate)
            } else {
                $text = '{0}  {1}/s' -f (Format-Size $got), (Format-Size $rate)
            }
            Write-Line "  $text"
        }
    } finally {
        $out.Close()
        $in.Close()
        $response.Close()
        if ($Interactive) { Clear-Line }
    }

    $elapsed = [int]((Get-Date) - $started).TotalSeconds
    Write-Ok ('downloaded {0} in {1}s' -f (Format-Size $got), $elapsed)
}

# ---------------------------------------------------------------- main

$arch = Get-Arch

if ($env:ZTORRENT_VERSION) {
    $api = "https://api.github.com/repos/$Repo/releases/tags/$($env:ZTORRENT_VERSION)"
} else {
    $api = "https://api.github.com/repos/$Repo/releases/latest"
}

Write-Step 'looking up the latest release'
try {
    $release = Invoke-GitHub $api
} catch {
    Stop-WithError "could not reach the GitHub API for $Repo - is the release published? ($($_.Exception.Message))"
}

$tag = $release.tag_name
if (-not $tag) { Stop-WithError "no release found for $Repo" }

# Windows on ARM runs x64 under emulation, so a release without an arm64 build is
# still installable -- prefer the native one, fall back rather than refuse.
$wanted = @("-win-$arch.exe")
if ($arch -eq 'arm64') { $wanted += '-win-x64.exe' }

$asset = $null
$chosen = $null
foreach ($suffix in $wanted) {
    $asset = $release.assets | Where-Object { $_.name -like "*$suffix" } | Select-Object -First 1
    if ($asset) { $chosen = $suffix; break }
}
if (-not $asset) { Stop-WithError "the $tag release has no Windows $arch installer (.exe) attached" }

Write-Ok "ztorrent $tag - Windows $arch"
if ($chosen -ne "-win-$arch.exe") {
    Write-Note "no arm64 build in $tag - installing the x64 one, which runs under emulation"
}

$work = Join-Path ([System.IO.Path]::GetTempPath()) ('ztorrent-install-' + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work -Force | Out-Null
$installer = Join-Path $work $asset.name

try {
    Save-Asset -Uri $asset.browser_download_url -Path $installer -Total $asset.size

    # GitHub publishes the asset's SHA-256 as "sha256:<hex>". Checking it is the
    # part that clicking through a browser warning never does for you.
    if ($asset.digest -and $asset.digest -match '^sha256:([0-9a-fA-F]{64})$') {
        $expected = $Matches[1]
        $actual = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash
        if ($actual -ne $expected) {
            Stop-WithError "checksum mismatch - expected $expected, got $actual. Not installing."
        }
        Write-Ok 'checksum verified'
    } else {
        Write-Note 'no checksum published for this asset - skipping verification'
    }

    # /S is the NSIS silent switch, the same one the in-app updater uses. The
    # installer is built in currentUser mode, so this needs no administrator.
    Write-Step 'installing'
    $process = Start-Process -FilePath $installer -ArgumentList '/S' -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        Stop-WithError "the installer exited with code $($process.ExitCode)"
    }
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Ok "installed ztorrent $tag"
Write-Host ''
Write-Note 'Open it from the Start menu.'
Write-Host ''
