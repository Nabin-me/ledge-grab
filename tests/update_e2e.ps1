# End-to-end test of Windows auto-update against a local feed.
#   update_e2e.ps1 -Payload DIR -Out DIR
# Payload (built with a throwaway key, see the comment at the bottom):
#   LedgeGrab-0.1.2-win-x64.exe, LedgeGrab-0.1.99-win-x64.exe, LedgeGrab-Setup-<v>-x64.exe,
#   feed-good / feed-tamper / feed-badfile, each laid out like GitHub releases.
# Kills any running Ledge Grab first. Uninstalls the test install at the end.
param([Parameter(Mandatory)] [string]$Payload, [Parameter(Mandatory)] [string]$Out)
$ErrorActionPreference = 'Stop'
if ((Split-Path $Out -Leaf) -notlike 'lg-*') { throw "Out must be an lg-* folder" }
Remove-Item $Out -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory $Out | Out-Null
$results = Join-Path $Out 'results.txt'
function Result($ok, $what) { $line = "$(if ($ok) { 'PASS' } else { 'FAIL' }) $what"; Add-Content $results $line; Write-Host $line }
function Hash($p) { if (Test-Path $p) { (Get-FileHash $p -Algorithm SHA256).Hash } else { '' } }
function WaitLog($log, $pattern, $secs) {
    $end = (Get-Date).AddSeconds($secs)
    while ((Get-Date) -lt $end) {
        if ((Test-Path $log) -and (Select-String -Path $log -Pattern $pattern -Quiet)) { return $true }
        Start-Sleep -Milliseconds 250
    }
    return $false
}
function StopAll { Get-Process ledge-grab -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep -Milliseconds 500 }

# A tiny static file server on 127.0.0.1 (no URL ACL needed), one per feed.
$server = @'
param($root, $port)
$l = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, $port); $l.Start()
while ($true) {
  $c = $l.AcceptTcpClient(); $s = $c.GetStream(); $r = [IO.StreamReader]::new($s)
  $line = $r.ReadLine(); while (($h = $r.ReadLine()) -ne '' -and $h -ne $null) {}
  $path = [Uri]::UnescapeDataString(($line -split ' ')[1]).TrimStart('/') -replace '/', '\'
  $f = Join-Path $root $path
  if ($path -notmatch '\.\.' -and (Test-Path $f -PathType Leaf)) {
    $b = [IO.File]::ReadAllBytes($f); $hd = "HTTP/1.1 200 OK`r`nContent-Length: $($b.Length)`r`nConnection: close`r`n`r`n"
  } else { $b = [byte[]]@(); $hd = "HTTP/1.1 404 Not Found`r`nContent-Length: 0`r`nConnection: close`r`n`r`n" }
  $hb = [Text.Encoding]::ASCII.GetBytes($hd); $s.Write($hb, 0, $hb.Length); $s.Write($b, 0, $b.Length); $s.Flush(); $c.Close()
}
'@
Set-Content (Join-Path $Out 'serve.ps1') $server
$ports = @{ good = 8765; tamper = 8766; badfile = 8767 }
$servers = foreach ($k in $ports.Keys) {
    Start-Process powershell -WindowStyle Hidden -PassThru -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $Out 'serve.ps1'), (Join-Path $Payload "feed-$k"), $ports[$k]
}
Start-Sleep 2

$old = Join-Path $Payload 'LedgeGrab-0.1.2-win-x64.exe'
$new = Join-Path $Payload 'LedgeGrab-0.1.99-win-x64.exe'

function Launch($exe, $feed, $name) {
    $env:LEDGE_GRAB_UPDATE_FEED = "http://127.0.0.1:$($ports[$feed])"
    $env:LEDGE_GRAB_UPDATE_EVERY_SECS = '3'
    $env:LEDGE_GRAB_LOG = Join-Path $Out "$name.log"
    $env:LEDGE_GRAB_DIR = Join-Path $Out "$name-data"
    Start-Process $exe | Out-Null
}

try {
    StopAll
    # 1. Portable copy updates itself by swapping its exe.
    $dir = Join-Path $Out 'portable'; New-Item -ItemType Directory $dir | Out-Null
    $exe = Join-Path $dir 'ledge-grab.exe'; Copy-Item $old $exe
    Launch $exe 'good' 'portable'
    $log = Join-Path $Out 'portable.log'
    Result (WaitLog $log 'Ledge Grab 0.1.2 started' 15) 'portable: old version starts'
    Result (WaitLog $log 'update 0.1.99 ready \(Portable\)' 60) 'portable: finds, downloads and verifies 0.1.99'
    Result (WaitLog $log 'exe swapped, relaunching' 60) 'portable: installs when idle'
    Result (WaitLog $log 'Ledge Grab 0.1.99 started' 30) 'portable: relaunches as 0.1.99'
    Result ((Hash $exe) -eq (Hash $new)) 'portable: exe on disk is the new build'
    Start-Sleep 6
    Result (-not (Test-Path "$exe.old")) 'portable: old exe removed after relaunch'
    Result (@(Get-Process ledge-grab -ErrorAction SilentlyContinue).Count -eq 1) 'portable: exactly one copy running'
    Start-Sleep 8
    Result (-not (Select-String -Path $log -Pattern 'update 0.1.99 ready' -SimpleMatch | Select-Object -Skip 1)) 'portable: new version does not update again'
    StopAll

    # 2. Tampered manifest: refused, nothing changes.
    $dir = Join-Path $Out 'tamper'; New-Item -ItemType Directory $dir | Out-Null
    $exe = Join-Path $dir 'ledge-grab.exe'; Copy-Item $old $exe
    Launch $exe 'tamper' 'tamper'
    $log = Join-Path $Out 'tamper.log'
    Result (WaitLog $log "update check: signature doesn't match" 30) 'tamper: signature mismatch is reported'
    Start-Sleep 5
    Result ((Hash $exe) -eq (Hash $old) -and -not (Select-String -Path $log -Pattern 'ready' -Quiet)) 'tamper: nothing downloaded or installed'
    StopAll

    # 3. Wrong file bytes behind a valid manifest: refused.
    $dir = Join-Path $Out 'badfile'; New-Item -ItemType Directory $dir | Out-Null
    $exe = Join-Path $dir 'ledge-grab.exe'; Copy-Item $old $exe
    Launch $exe 'badfile' 'badfile'
    $log = Join-Path $Out 'badfile.log'
    Result (WaitLog $log "update check: download doesn't match the signed checksum" 30) 'badfile: checksum mismatch is reported'
    Start-Sleep 5
    Result ((Hash $exe) -eq (Hash $old) -and -not (Test-Path (Join-Path $Out 'badfile-data\updates\LedgeGrab-0.1.99-win-x64.exe'))) 'badfile: bad download deleted, exe unchanged'
    StopAll

    # 4. Installed copy (per-user installer) runs the new installer silently.
    $setupOld = Join-Path $Payload 'LedgeGrab-Setup-0.1.2-x64.exe'
    Start-Process -Wait $setupOld -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', '/TASKS=', '/SP-'
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{8F3B9C1A-4D2E-4E6B-9A73-2C1F0E5D8B41}_is1'
    $loc = (Get-ItemProperty $key).InstallLocation
    $exe = Join-Path $loc 'ledge-grab.exe'
    Result ((Get-ItemProperty $key).DisplayVersion -eq '0.1.2') 'installed: 0.1.2 installed for this user'
    Launch $exe 'good' 'installed'
    $log = Join-Path $Out 'installed.log'
    Result (WaitLog $log 'update 0.1.99 ready \(UserInstall\)' 60) 'installed: picks the installer'
    Result (WaitLog $log 'installer started: true' 60) 'installed: installs when idle'
    Result (WaitLog $log 'Ledge Grab 0.1.99 started' 90) 'installed: relaunches as 0.1.99'
    Result ((Get-ItemProperty $key).DisplayVersion -eq '0.1.99') 'installed: Apps list shows 0.1.99'
    Result ((Hash $exe) -eq (Hash $new)) 'installed: installed exe is the new build'
    Result (@(Get-Process ledge-grab -ErrorAction SilentlyContinue).Count -eq 1) 'installed: exactly one copy running'
} finally {
    StopAll
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{8F3B9C1A-4D2E-4E6B-9A73-2C1F0E5D8B41}_is1'
    if (Test-Path $key) {
        $u = (Get-ItemProperty $key).UninstallString -replace '"', ''
        if ($u) { Start-Process -Wait $u -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' }
    }
    $servers | Stop-Process -Force -ErrorAction SilentlyContinue
    Set-Content (Join-Path $Out 'done.flag') 'done'
}
# Build the payload with a throwaway key: compile with LEDGE_GRAB_UPDATES=1 and
# LEDGE_GRAB_UPDATE_PUBKEY=<test public key>, the new one also with
# LEDGE_GRAB_VERSION_OVERRIDE=0.1.99, then sign with packaging/update-manifest.py.
