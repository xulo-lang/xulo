# Install a prebuilt Xulo release from GitHub on Windows.
#
#   irm https://github.com/xulo-lang/xulo/releases/latest/download/install.ps1 | iex
#
# Environment variables:
#   XULO_VERSION   release version to install (default: latest), e.g. 0.1.0
#   XULO_PREFIX    install prefix (default: $env:LOCALAPPDATA\Xulo)

$ErrorActionPreference = "Stop"

$Repo = "xulo-lang/xulo"
$Version = if ($env:XULO_VERSION) { $env:XULO_VERSION } else { "latest" }
$Prefix = if ($env:XULO_PREFIX) { $env:XULO_PREFIX } else { Join-Path $env:LOCALAPPDATA "Xulo" }

function Fail($Message) {
    throw "install.ps1: $Message"
}

$Arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
switch ($Arch) {
    "X64" { $Triple = "x86_64-pc-windows-msvc" }
    # Windows on ARM runs the x64 build under emulation.
    "Arm64" { $Triple = "x86_64-pc-windows-msvc" }
    default { Fail "unsupported architecture: $Arch" }
}

if ($Version -eq "latest") {
    $Release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
    $Tag = $Release.tag_name
    if (-not $Tag) { Fail "could not determine the latest release" }
} elseif ($Version.StartsWith("v")) {
    $Tag = $Version
} else {
    $Tag = "v$Version"
}

$Number = $Tag.TrimStart("v")
$Asset = "xulo-$Number-$Triple.zip"
$Base = "https://github.com/$Repo/releases/download/$Tag"
$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("xulo-install-" + [System.Guid]::NewGuid())

try {
    New-Item -ItemType Directory -Path $Tmp | Out-Null

    Write-Host "Downloading Xulo $Number..."
    Invoke-WebRequest -Uri "$Base/$Asset" -OutFile (Join-Path $Tmp $Asset)
    Invoke-WebRequest -Uri "$Base/SHA256SUMS" -OutFile (Join-Path $Tmp "SHA256SUMS")

    $Expected = (Get-Content (Join-Path $Tmp "SHA256SUMS") |
        Where-Object { $_ -match "\s$([regex]::Escape($Asset))$" } |
        ForEach-Object { ($_ -split "\s+")[0] } |
        Select-Object -First 1)
    if (-not $Expected) { Fail "checksum for $Asset is missing" }
    $Actual = (Get-FileHash (Join-Path $Tmp $Asset) -Algorithm SHA256).Hash.ToLower()
    if ($Actual -ne $Expected) { Fail "checksum verification failed" }

    Expand-Archive -Path (Join-Path $Tmp $Asset) -DestinationPath $Tmp
    $Root = Join-Path $Tmp "xulo-$Number-$Triple"

    New-Item -ItemType Directory -Force -Path (Join-Path $Prefix "bin") | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $Prefix "lib\xulo") | Out-Null
    foreach ($Bin in @("xulo.exe", "xupm.exe", "xulo-analyzer.exe")) {
        $Source = Join-Path $Root "bin\$Bin"
        if (Test-Path $Source) {
            Copy-Item $Source (Join-Path $Prefix "bin\$Bin") -Force
        }
    }
    Copy-Item (Join-Path $Root "lib\xulo\xulo_runtime.lib") (Join-Path $Prefix "lib\xulo\xulo_runtime.lib") -Force

    Write-Host "Installed Xulo $Number to $(Join-Path $Prefix 'bin')"
    $BinDir = Join-Path $Prefix "bin"
    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if (-not (($UserPath -split ";") -contains $BinDir)) {
        Write-Host "Add $BinDir to PATH to use the Xulo tools."
    }
} finally {
    Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
