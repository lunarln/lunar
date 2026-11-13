# Install the Lunar compiler from a GitHub release, on Windows (x64):
#
#   irm https://raw.githubusercontent.com/lunarln/lunar/main/install.ps1 | iex
#   .\install.ps1 [-Prefix DIR] [-Version X.Y.Z]
#
# It downloads the release archive, checks it against its SHA-256 checksum
# and unpacks it into the prefix (default ~\.lunar, or $env:LUNAR_PREFIX):
# the compiler and LLVM-C.dll in bin\, its standard library in lib\. The
# version defaults to the latest release (or $env:LUNAR_VERSION).
param(
    [string]$Prefix = $(if ($env:LUNAR_PREFIX) { $env:LUNAR_PREFIX } else { Join-Path $HOME '.lunar' }),
    [string]$Version = $env:LUNAR_VERSION
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$repo = 'lunarln/lunar'
$target = 'x86_64-pc-windows-msvc'

$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if ($arch -ne 'X64' -and $arch -ne 'Arm64') {
    throw "there is no prebuilt Lunar for Windows on $arch; build it from source (see https://github.com/$repo#building)"
}

if (-not $Version) {
    $latest = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest"
    $Version = $latest.tag_name
}
$Version = $Version.TrimStart('v')

$name = "lunar-$Version-$target"
$archive = "$name.zip"
$url = "https://github.com/$repo/releases/download/v$Version/$archive"
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("lunar-install-" + [System.Guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
    Write-Host "downloading Lunar $Version for $target"
    Invoke-WebRequest $url -OutFile (Join-Path $tmp $archive) -UseBasicParsing
    Invoke-WebRequest "$url.sha256" -OutFile (Join-Path $tmp "$archive.sha256") -UseBasicParsing

    $expected = ((Get-Content (Join-Path $tmp "$archive.sha256") -Raw).Trim() -split '\s+')[0].ToLower()
    $actual = (Get-FileHash (Join-Path $tmp $archive) -Algorithm SHA256).Hash.ToLower()
    if ($expected -ne $actual) {
        throw "checksum mismatch for ${archive}: got $actual, expected $expected"
    }

    Expand-Archive (Join-Path $tmp $archive) -DestinationPath $tmp
    $src = Join-Path $tmp $name
    if (-not (Test-Path (Join-Path $src 'bin\lunar.exe'))) {
        throw "$archive does not have bin\lunar.exe"
    }

    # Replace the standard library (files may have been removed); the rest
    # is overwritten in place.
    $bin = Join-Path $Prefix 'bin'
    $doc = Join-Path $Prefix 'share\doc\lunar'
    New-Item -ItemType Directory -Force $bin, (Join-Path $Prefix 'lib'), $doc | Out-Null
    $std = Join-Path $Prefix 'lib\lunar'
    if (Test-Path $std) { Remove-Item -Recurse -Force $std }
    try {
        Copy-Item -Force (Join-Path $src 'bin\*') $bin
    } catch {
        throw "could not replace $bin\lunar.exe; is it running (an editor's language server)? $_"
    }
    Copy-Item -Recurse -Force (Join-Path $src 'lib\*') (Join-Path $Prefix 'lib')
    Copy-Item -Recurse -Force (Join-Path $src 'LICENSE-*'), (Join-Path $src 'README.md'), (Join-Path $src 'licenses') $doc
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

$installed = & (Join-Path $bin 'lunar.exe') --version
Write-Host "installed $installed to $Prefix"

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not (($userPath -split ';') -contains $bin) -and -not (($env:PATH -split ';') -contains $bin)) {
    Write-Host ""
    Write-Host "Add $bin to your PATH. For your user, from PowerShell:"
    Write-Host ""
    Write-Host "    [Environment]::SetEnvironmentVariable('Path', `"$bin;`" + [Environment]::GetEnvironmentVariable('Path', 'User'), 'User')"
    Write-Host ""
    Write-Host "then open a new terminal."
}

# Programs are linked by LLVM's clang (with the MSVC libraries) or by MSVC's
# link.exe from a developer prompt.
if (-not (Get-Command clang -ErrorAction SilentlyContinue) -and -not (Get-Command link.exe -ErrorAction SilentlyContinue | Where-Object { $_.Source -notmatch '\\Git\\' }) -and -not $env:LUNAR_CC) {
    Write-Host ""
    Write-Host "note: lunar links programs with LLVM clang or MSVC link.exe, and neither is on PATH."
    Write-Host "      Install the Visual Studio Build Tools (the C++ workload) and LLVM (https://github.com/llvm/llvm-project/releases),"
    Write-Host "      or run lunar from a Developer PowerShell for Visual Studio."
}
