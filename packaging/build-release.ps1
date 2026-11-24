<#
.SYNOPSIS
    Builds Ledge Grab release executables, portable ZIP, Windows Setup installer (.exe), optional Authenticode signing, and SHA256 checksums.
.EXAMPLE
    .\packaging\build-release.ps1
    .\packaging\build-release.ps1 -Sign
#>
param(
    [string]$AppName = "Ledge Grab",
    [string]$FileSlug = "LedgeGrab",
    [string]$BinName = "",
    [string]$IconPath = "assets\ledge-grab.ico",
    [switch]$Sign,
    [string]$CertThumbprint = "",
    [string]$TimestampUrl = "http://timestamp.digicert.com",
    [switch]$SkipCargoBuild
)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$DistDir = Join-Path $RepoRoot "dist"

# Ensure cargo is on PATH
$CargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if (Test-Path $CargoBin) {
    $env:PATH = "$CargoBin;$env:PATH"
}

# Read package name and version from Cargo.toml
$CargoToml = Get-Content (Join-Path $RepoRoot "Cargo.toml") -Raw
$PkgName = ([regex]::Match($CargoToml, '(?m)^name\s*=\s*"([^"]+)"')).Groups[1].Value
$Version = ([regex]::Match($CargoToml, '(?m)^version\s*=\s*"([^"]+)"')).Groups[1].Value
if (-not $BinName) { $BinName = $PkgName }

Write-Host "==> Building $AppName v$Version (binary: $BinName.exe)" -ForegroundColor Cyan

if (-not $SkipCargoBuild) {
    Push-Location $RepoRoot
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed" }
    } finally {
        Pop-Location
    }
}

$ReleaseExe = Join-Path $RepoRoot "target\release\$BinName.exe"
if (-not (Test-Path $ReleaseExe)) {
    throw "Release binary not found at $ReleaseExe"
}

New-Item -ItemType Directory -Force -Path $DistDir | Out-Null

# Locate signtool.exe & certificate if -Sign is requested
$SignTool = $null
if ($Sign) {
    $SignTool = (Get-Command "signtool.exe" -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source)
    if (-not $SignTool) {
        $SignTool = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin\" -Recurse -Filter "signtool.exe" -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
            Sort-Object FullName -Descending |
            Select-Object -First 1 -ExpandProperty FullName
    }
    if (-not $SignTool) { throw "signtool.exe not found in PATH or Windows 10/11 SDK." }

    if (-not $CertThumbprint) {
        $cert = Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert |
            Where-Object { $_.Subject -like "CN=$AppName*" } |
            Sort-Object NotAfter -Descending |
            Select-Object -First 1
        if (-not $cert) {
            Write-Host "  [..] Creating local self-signed Authenticode cert for $AppName..." -ForegroundColor Yellow
            $cert = New-SelfSignedCertificate -Type CodeSigningCert `
                -Subject "CN=$AppName (Local Dev)" `
                -FriendlyName "$AppName Local Code Signing" `
                -KeyAlgorithm RSA -KeyLength 3072 -HashAlgorithm SHA256 `
                -NotAfter (Get-Date).AddYears(5) `
                -CertStoreLocation "Cert:\CurrentUser\My"
        }
        $CertThumbprint = $cert.Thumbprint
        $PublicCer = Join-Path $DistDir "$FileSlug-LocalCert.cer"
        Export-Certificate -Cert $cert -FilePath $PublicCer -Force | Out-Null
        Write-Host "  [OK] Signing Cert   : $($cert.Subject) ($CertThumbprint)" -ForegroundColor Green
    }
}

function Invoke-SignFile([string]$FilePath) {
    $tsUrls = @($TimestampUrl, "http://timestamp.sectigo.com") | Select-Object -Unique
    foreach ($ts in $tsUrls) {
        & $SignTool sign /sha1 $CertThumbprint /fd SHA256 /tr $ts /td SHA256 /d $AppName $FilePath | Out-Null
        if ($LASTEXITCODE -eq 0) { return }
    }
    & $SignTool sign /sha1 $CertThumbprint /fd SHA256 /d $AppName $FilePath
    if ($LASTEXITCODE -ne 0) { throw "signtool failed for $FilePath" }
}

# 1. Standalone single-file executable (signed first so both ZIP & installer embed the signed binary)
$StandaloneExe = Join-Path $DistDir "$FileSlug-$Version-win-x64.exe"
Copy-Item $ReleaseExe $StandaloneExe -Force
if ($Sign) {
    Invoke-SignFile $StandaloneExe
    Write-Host "  [OK] Signed EXE     : $StandaloneExe" -ForegroundColor Green
} else {
    Write-Host "  [OK] Standalone EXE : $StandaloneExe" -ForegroundColor Green
}

# 2. Portable ZIP package (embeds the signed binary when -Sign is set)
$StageDir = Join-Path $DistDir "_portable_stage"
if (Test-Path $StageDir) { Remove-Item $StageDir -Recurse -Force }
New-Item -ItemType Directory -Force -Path $StageDir | Out-Null
Copy-Item $StandaloneExe (Join-Path $StageDir "$BinName.exe") -Force
Copy-Item (Join-Path $RepoRoot "README.md") (Join-Path $StageDir "README.md") -Force
if (Test-Path (Join-Path $RepoRoot "LICENSE")) {
    Copy-Item (Join-Path $RepoRoot "LICENSE") (Join-Path $StageDir "LICENSE.txt") -Force
}
if (Test-Path (Join-Path $RepoRoot "assets\OFL.txt")) {
    Copy-Item (Join-Path $RepoRoot "assets\OFL.txt") (Join-Path $StageDir "LICENSE-FONT.txt") -Force
}
$PortableZip = Join-Path $DistDir "$FileSlug-$Version-win-x64-portable.zip"
if (Test-Path $PortableZip) { Remove-Item $PortableZip -Force }
Compress-Archive -Path (Join-Path $StageDir "*") -DestinationPath $PortableZip -CompressionLevel Optimal
Remove-Item $StageDir -Recurse -Force
Write-Host "  [OK] Portable ZIP   : $PortableZip" -ForegroundColor Green

# 3. Windows Setup Installer (.exe) via Inno Setup 6
$IsccCandidates = @(
    (Get-Command "ISCC.exe" -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source),
    (Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 6\ISCC.exe"),
    "C:\Program Files (x86)\Inno Setup 6\ISCC.exe",
    "C:\Program Files\Inno Setup 6\ISCC.exe"
) | Where-Object { $_ -and (Test-Path $_) }

$Iscc = $IsccCandidates | Select-Object -First 1
if (-not $Iscc) {
    throw "Inno Setup 6 (ISCC.exe) not found. Install it via: winget install JRSoftware.InnoSetup"
}

$IssPath = Join-Path $RepoRoot "packaging\windows\installer.iss"
$IsccArgs = @(
    "/DAppName=$AppName",
    "/DAppFileSlug=$FileSlug",
    "/DAppExeName=$BinName.exe",
    "/DAppVersion=$Version",
    "/DAppPublisher=$AppName",
    "/DAppIcon=$IconPath",
    "/DAppMutexName=Local\$FileSlug",
    "/DSourceExe=$StandaloneExe",
    "/DOutputDirPath=$DistDir"
)
if ($Sign) {
    $IsccArgs += "/DEnableSigning=1"
    $IsccArgs += "/Ssigntool=`$q$SignTool`$q sign /sha1 $CertThumbprint /fd SHA256 /tr $TimestampUrl /td SHA256 /d `$q$AppName Setup`$q `$f"
}
$IsccArgs += $IssPath

& $Iscc @IsccArgs
if ($LASTEXITCODE -ne 0) { throw "ISCC installer compilation failed" }

$SetupExe = Join-Path $DistDir "$FileSlug-Setup-$Version-x64.exe"
Write-Host "  [OK] Setup Installer: $SetupExe" -ForegroundColor Green

# 4. SHA256SUMS.txt
$ChecksumFile = Join-Path $DistDir "SHA256SUMS.txt"
$Artifacts = @($SetupExe, $PortableZip, $StandaloneExe)
$Lines = foreach ($f in $Artifacts) {
    $h = (Get-FileHash $f -Algorithm SHA256).Hash.ToLowerInvariant()
    "$h  $(Split-Path $f -Leaf)"
}
$Lines | Set-Content -Path $ChecksumFile -Encoding UTF8
Write-Host "  [OK] Checksums      : $ChecksumFile" -ForegroundColor Green

Write-Host "`nRelease artifacts in $DistDir`:" -ForegroundColor Cyan
Get-ChildItem $DistDir | Select-Object Name, @{N='SizeMB';E={[math]::Round($_.Length/1MB, 2)}} | Format-Table -AutoSize
