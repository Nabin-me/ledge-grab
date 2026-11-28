# Windows desktop end-to-end checks for the paste queue, failed saves, and the
# collapsed-ledge indicator. Must run in the signed-in user's session (e.g. a
# scheduled task with /IT); it types into real windows.
#   powershell -File tests\queue_e2e.ps1 -Exe target\debug\ledge-grab.exe -Out C:\tmp\lg-e2e
param([string]$Exe, [string]$Out, [string[]]$Only = @())
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force $Out | Out-Null
$global:LogFile = Join-Path $Out "e2e.log"
Remove-Item $global:LogFile -ErrorAction SilentlyContinue
. (Join-Path $PSScriptRoot "win_driver.ps1")
$Exe = (Resolve-Path $Exe).Path
$results = New-Object Collections.ArrayList

$red = [Drawing.Color]::FromArgb(230, 20, 20); $green = [Drawing.Color]::FromArgb(20, 210, 40); $blue = [Drawing.Color]::FromArgb(20, 40, 230)
$page = "file:///" + ((Join-Path $PSScriptRoot "paste_target.html") -replace "\\", "/")

function Result($name, $ok, $detail) {
    $line = "{0} {1}: {2}" -f ($(if ($ok) { "PASS" } else { "FAIL" })), $name, $detail
    Log $line; [void]$results.Add($line)
}

function Saved-Count($ledge) {
    $log = Join-Path $ledge.Data "log.txt"
    if (Test-Path $log) { @(Select-String -Path $log -Pattern "^saved_ms .* true").Count } else { 0 }
}

# Three shelf images in display order A, B, C (the newest shows first, so copy C, B, A).
function Seed-ABC($ledge) {
    foreach ($x in @(@($blue, "C"), @($green, "B"), @($red, "A"))) {
        Copy-Solid $x[0] $x[1]
        Start-Sleep -Milliseconds 450
    }
    Wait-Until { (Saved-Count $ledge) -ge 3 } 6000 "three shelf saves" | Out-Null
}

function Open-Shelf($ledge) {
    Post $ledge $WM.MOVE 105 4
    Start-Sleep -Milliseconds 900
    $wheel = ((-120) -band 0xFFFF) -shl 16
    Post $ledge $WM.WHEEL 200 50 $wheel
    Start-Sleep -Milliseconds 700
}

# Tick tiles out of order on purpose: the queue must follow display order.
function Start-Sequential($ledge, $shot = $false) {
    Open-Shelf $ledge
    foreach ($i in 2, 0, 1) { Ledge-Click $ledge (133 + 142 * $i) 31 }
    if ($shot) {
        Ledge-Shot $ledge (Join-Path $Out "01b-shelf-selected.png")
        Ledge-Click $ledge 224 134   # More: shows the Sequential Copy action
        Ledge-Shot $ledge (Join-Path $Out "01c-shelf-more.png")
    }
    Post $ledge $WM.KEYDOWN 0 0 $VK.Q
    Start-Sleep -Milliseconds 700
}

function Start-Edge {
    $profile = Join-Path $env:TEMP ("lg-edge-" + [guid]::NewGuid().ToString("N"))
    $edge = "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
    $p = Start-Process $edge -ArgumentList "--user-data-dir=$profile", "--inprivate", "--no-first-run", "--no-default-browser-check", "--disable-sync", "--disable-features=msEdgeSidebarV2", "--new-window", "--window-position=150,250", "--window-size=1000,700", $page -PassThru
    $h = Wait-Until { ([Drv]::Find(0, $null, "PASTE TARGET READY"))[0] } 15000 "Edge paste target"
    Start-Sleep -Milliseconds 1200
    return @{ H = $h; Profile = $profile }
}
function Close-Window($h) { [void][Drv]::PostMessage($h, $WM.CLOSE, [IntPtr]0, [IntPtr]0) }
function Focus-Target($h) {
    $r = [Drv]::Rect($h)
    [Drv]::Click([int](($r.L + $r.R) / 2), [int]($r.T + ($r.B - $r.T) * 0.45))
    Start-Sleep -Milliseconds 300
}
function Pastes($h) { $t = [Drv]::Title($h); if ($t.StartsWith("PASTES:")) { ($t.Substring(7) -split " ")[0] } else { "" } }
function Wait-Pastes($h, $n, $ms = 6000) {
    try { Wait-Until { $p = Pastes $h; if ($p -and @($p -split ",").Count -ge $n -and $p -notmatch "…") { $p } } $ms "$n pastes" }
    catch { Pastes $h }
}

function Scenario-EdgeRapid {
    $l = Start-Ledge $Exe (Join-Path $Out "data-edge")
    try {
        Seed-ABC $l
        Ledge-Shot $l (Join-Path $Out "01-rest-indicator.png")
        Result "collapsed indicator size" ((Ledge-Size $l)[1] -le 10) ("rest " + ((Ledge-Size $l) -join "x"))
        Start-Sequential $l $true
        $size = Ledge-Size $l
        Ledge-Shot $l (Join-Path $Out "02-queue-next-1-of-3.png")
        Result "Sequential Copy shows the queue ledge" ($size[0] -eq 330) ("size " + ($size -join "x"))
        $e = Start-Edge
        Focus-Target $e.H
        Paste-Chord 3 30
        $got = Wait-Pastes $e.H 3
        $r = [Drv]::Rect($e.H)
        [Drv]::Capture($r.L, $r.T, $r.R, $r.B, (Join-Path $Out "02b-edge-after-paste.png"))
        Log ("edge title: " + [Drv]::Title($e.H))
        Result "Edge: three rapid Ctrl+V paste A,B,C" ($got -eq "A,B,C") "got '$got'"
        Start-Sleep -Milliseconds 400
        Ledge-Shot $l (Join-Path $Out "03-after-queue.png")
        Result "queue ends after the last image" ((Ledge-Size $l)[1] -le 10 -or (Ledge-Size $l)[0] -eq 84) ("size " + ((Ledge-Size $l) -join "x"))
        Close-Window $e.H
    } finally { Stop-Ledge $l; Copy-Item (Join-Path $l.Data "log.txt") (Join-Path $Out "edge-ledge-log.txt") -ErrorAction SilentlyContinue }
}

function Scenario-WinForms {
    $l = Start-Ledge $Exe (Join-Path $Out "data-winforms")
    try {
        Seed-ABC $l
        Start-Sequential $l
        $t = Start-Process powershell -ArgumentList "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", (Join-Path $PSScriptRoot "winforms_target.ps1") -PassThru
        $h = Wait-Until { ([Drv]::Find([uint32]$t.Id, $null, "WINFORMS TARGET READY"))[0] } 15000 "WinForms target"
        Start-Sleep -Milliseconds 500
        Focus-Target $h
        Paste-Chord 3 25
        $got = Wait-Pastes $h 3
        Result "WinForms (CF_BITMAP): three rapid Ctrl+V paste A,B,C" ($got -eq "A,B,C") "got '$got'"
        $t.Kill()
    } finally { Stop-Ledge $l; Copy-Item (Join-Path $l.Data "log.txt") (Join-Path $Out "winforms-ledge-log.txt") -ErrorAction SilentlyContinue }
}

function Scenario-Spaced {
    $l = Start-Ledge $Exe (Join-Path $Out "data-spaced")
    try {
        Seed-ABC $l
        Start-Sequential $l
        $e = Start-Edge
        Focus-Target $e.H
        foreach ($n in 1..3) { Paste-Chord 1 0; Start-Sleep -Milliseconds 700 }
        $got = Wait-Pastes $e.H 3
        Result "Edge: spaced Ctrl+V paste A,B,C" ($got -eq "A,B,C") "got '$got'"
        Close-Window $e.H
    } finally { Stop-Ledge $l }
}

# A paste that nobody reads (text-only target) must stop and offer Next, not advance.
function Scenario-StallAndNext {
    $l = Start-Ledge $Exe (Join-Path $Out "data-stall")
    try {
        Seed-ABC $l
        Start-Sequential $l
        $np = Start-Process notepad -PassThru
        Start-Sleep -Milliseconds 1500
        $nh = Wait-Until { [Drv]::GetForegroundWindow() } 3000 "notepad"
        Paste-Chord 1 0
        Start-Sleep -Milliseconds 2200
        Ledge-Shot $l (Join-Path $Out "04-paste-not-detected.png")
        $log = Get-Content (Join-Path $l.Data "log.txt") -Raw
        Result "unread paste stalls instead of advancing" ($log -match "queue stalled .?NotRead" -and $log -notmatch "queue placed") "log has stall: $($log -match 'queue stalled .?NotRead')"
        Close-Window $nh; Start-Sleep -Milliseconds 400
        Get-Process notepad -ErrorAction SilentlyContinue | Where-Object { $_.StartTime -gt (Get-Date).AddMinutes(-2) } | Stop-Process -Force -ErrorAction SilentlyContinue
        # Manual advance: Next moves to B.
        Ledge-Click $l 255 24
        Start-Sleep -Milliseconds 300
        Ledge-Shot $l (Join-Path $Out "05-after-next.png")
        $e = Start-Edge
        Focus-Target $e.H
        Paste-Chord 1 0
        $got = Wait-Pastes $e.H 1
        Result "Next advances to B; the following paste is B" ($got -eq "B") "got '$got'"
        Close-Window $e.H
    } finally { Stop-Ledge $l; Copy-Item (Join-Path $l.Data "log.txt") (Join-Path $Out "stall-ledge-log.txt") -ErrorAction SilentlyContinue }
}

# A missing image is never skipped silently and A is never pasted twice.
function Scenario-Missing {
    $l = Start-Ledge $Exe (Join-Path $Out "data-missing")
    try {
        Seed-ABC $l
        Start-Sequential $l
        $files = @(Get-ChildItem -Recurse (Join-Path $l.Data "shots") -Filter *.png | Sort-Object CreationTime)
        # Copied C, B, A: the second file is B.
        Remove-Item $files[1].FullName
        $e = Start-Edge
        Focus-Target $e.H
        Paste-Chord 1 0; Start-Sleep -Milliseconds 900
        Ledge-Shot $l (Join-Path $Out "06-missing.png")
        Paste-Chord 1 0; Start-Sleep -Milliseconds 900
        $mid = Pastes $e.H
        Ledge-Click $l 255 24   # Skip
        Start-Sleep -Milliseconds 400
        Paste-Chord 1 0
        $got = Wait-Pastes $e.H 3 3000
        Result "missing B stops the queue; paste meanwhile is not A again; Skip then gives C" (($got -split ",")[0] -eq "A" -and $got -notmatch "A,A" -and $got -notmatch "B" -and $got.EndsWith("C")) "got '$got' (before Skip: '$mid')"
        Close-Window $e.H
    } finally { Stop-Ledge $l; Copy-Item (Join-Path $l.Data "log.txt") (Join-Path $Out "missing-ledge-log.txt") -ErrorAction SilentlyContinue }
}

# Copying something else elsewhere ends the queue.
function Scenario-ReplacedElsewhere {
    $l = Start-Ledge $Exe (Join-Path $Out "data-replaced")
    try {
        Seed-ABC $l
        Start-Sequential $l
        $before = Ledge-Size $l
        [Windows.Forms.Clipboard]::SetText("something else")
        Start-Sleep -Milliseconds 900
        $after = Ledge-Size $l
        Result "clipboard replaced elsewhere clears the queue" ($before[0] -eq 330 -and $after[1] -le 10) ("before " + ($before -join "x") + ", after " + ($after -join "x"))
    } finally { Stop-Ledge $l }
}

# A failed save leaves no tile and says so.
function Scenario-FailedSave {
    $l = Start-Ledge $Exe (Join-Path $Out "data-failsave") @{ LEDGE_GRAB_FAIL_SAVES = "1" }
    try {
        Copy-Solid $red "A"
        Start-Sleep -Milliseconds 900
        $notice = Ledge-Size $l
        Ledge-Shot $l (Join-Path $Out "07-save-failed-notice.png")
        Ledge-Click $l 100 30   # dismiss
        Start-Sleep -Milliseconds 500
        Post $l $WM.MOVE 105 4; Start-Sleep -Milliseconds 900
        $peek = Ledge-Size $l
        Ledge-Shot $l (Join-Path $Out "08-shelf-after-failed-save.png")
        Result "failed save: notice shown, shelf empty" ($notice[0] -eq 390 -and $peek[0] -eq 200 -and $peek[1] -eq 60) ("notice " + ($notice -join "x") + ", peek " + ($peek -join "x"))
    } finally { Stop-Ledge $l; Copy-Item (Join-Path $l.Data "log.txt") (Join-Path $Out "failsave-ledge-log.txt") -ErrorAction SilentlyContinue }
}

Stop-TestEdge
$all = "EdgeRapid", "WinForms", "Spaced", "StallAndNext", "Missing", "ReplacedElsewhere", "FailedSave"
foreach ($s in $all) {
    if ($Only.Count -and $Only -notcontains $s) { continue }
    Log "== $s"
    try { & "Scenario-$s" } catch { Result $s $false ("error: " + $_.Exception.Message + " @ " + $_.InvocationInfo.ScriptLineNumber) }
    Start-Sleep -Milliseconds 500
}
Stop-TestEdge
Log "== SUMMARY"
$results | ForEach-Object { Log $_ }
Set-Content -Path (Join-Path $Out "done.flag") -Value ($results -join "`n")
