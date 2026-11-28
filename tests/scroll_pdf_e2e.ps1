# Windows desktop checks for the Scroll to PDF preview: launch messages, a full run
# from message 06, Esc, covered and moving panes, and an uncertain seam. Run in the
# signed-in user's session; it moves the real mouse and scrolls a real Edge window.
#   powershell -File tests\scroll_pdf_e2e.ps1 -Exe target\debug\ledge-grab.exe -Out C:\tmp\lg-pdf
param([string]$Exe, [string]$Out, [string[]]$Only = @())
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force $Out | Out-Null
$global:LogFile = Join-Path $Out "pdf-e2e.log"
Remove-Item $global:LogFile -ErrorAction SilentlyContinue
. (Join-Path $PSScriptRoot "win_driver.ps1")
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$Exe = (Resolve-Path $Exe).Path
$results = New-Object Collections.ArrayList
$Title = "Ledge Grab Scroll to PDF Preview"
$RUN = 0x8000 + 2
$SCROLL_PDF = 12

function Result($name, $ok, $detail) {
    $line = "{0} {1}: {2}" -f ($(if ($ok) { "PASS" } else { "FAIL" })), $name, $detail
    Log $line; [void]$results.Add($line)
}

function Start-Exe([string]$data, [hashtable]$envs) {
    New-Item -ItemType Directory -Force $data | Out-Null
    $psi = New-Object Diagnostics.ProcessStartInfo $Exe
    $psi.UseShellExecute = $false
    $psi.EnvironmentVariables["LEDGE_GRAB_DIR"] = $data
    $psi.EnvironmentVariables["LEDGE_GRAB_LOG"] = (Join-Path $data "preview-launch.log")
    foreach ($k in $envs.Keys) { $psi.EnvironmentVariables[$k] = $envs[$k] }
    return [Diagnostics.Process]::Start($psi)
}

function Dialog($p) { ([Drv]::Find([uint32]$p.Id, "#32770", $Title))[0] }
function Ok-Dialog($h) {
    [void][Drv]::PostMessage($h, 0x111, [IntPtr]1, [IntPtr]0)
    Start-Sleep -Milliseconds 600
    if ([Drv]::IsWindow($h)) { Log "dialog still open after OK; closing it"; [void][Drv]::PostMessage($h, 0x10, [IntPtr]0, [IntPtr]0) }
}

function Start-Preview([string]$data) {
    Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
    $p = Start-Exe $data @{ LEDGE_GRAB_SCROLL_PDF_PREVIEW = "1" }
    $d = Wait-Until { Dialog $p } 10000 "preview dialog"
    $text = [Drv]::DialogText($d)
    return @{ P = $p; Dialog = $d; Text = $text; Data = $data }
}

function Sensor($p) { ([Drv]::Find([uint32]$p.Id, "LedgeGrabNotch", "Ledge Grab sensor"))[0] }
function Preview-Log($data) { $f = Join-Path $data "preview-launch.log"; if (Test-Path $f) { Get-Content $f -Raw } else { "" } }

function Scenario-StartupClean {
    $pv = Start-Preview (Join-Path $Out "data-startup")
    [Drv]::Capture(0, 0, 1400, 900, (Join-Path $Out "10-preview-ready.png"))
    Ok-Dialog $pv.Dialog
    Start-Sleep -Milliseconds 800
    $alive = -not $pv.P.HasExited
    Result "preview starts with no other Ledge Grab: says ready, ledge stays up" ($pv.Text -match "Preview is ready" -and $alive -and (Sensor $pv.P)) ("dialog '" + $pv.Text.Trim() + "', running=$alive")
    $pv.P.Kill()
}

function Scenario-StartupWhileRunning {
    # Stand in for the installed app: a regular instance (own data folder).
    $regular = Start-Exe (Join-Path $Out "data-regular") @{}
    Wait-Until { Sensor $regular } 8000 "regular instance" | Out-Null
    Start-Sleep -Milliseconds 500
    $pv = Start-Preview (Join-Path $Out "data-startup2")
    [Drv]::Capture(0, 0, 1400, 900, (Join-Path $Out "11-preview-already-running.png"))
    Ok-Dialog $pv.Dialog
    $exited = $pv.P.WaitForExit(4000)
    $code = if ($exited) { $pv.P.ExitCode } else { "running" }
    Result "preview while the app is running: explains what to do and exits" ($pv.Text -match "already running" -and $pv.Text -match "Quit" -and $exited -and $code -eq 2) ("dialog '" + $pv.Text.Trim() + "', exit=$code")
    $log = Preview-Log $pv.Data
    Result "launch log records the stop" ($log -match "Launch stopped at Ledge Grab is already running") ($log.Trim() -replace "`r?`n", " / ")
    $regular.Kill()
}

function Start-Fixture([string]$file, [int]$x = 160, [int]$y = 10) {
    $profile = Join-Path $env:TEMP ("lg-edge-" + [guid]::NewGuid().ToString("N"))
    $url = "file:///" + ((Join-Path $PSScriptRoot $file) -replace "\\", "/")
    $edge = "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
    Start-Process $edge -ArgumentList "--user-data-dir=$profile", "--inprivate", "--disable-sync", "--no-first-run", "--no-default-browser-check", "--new-window", "--force-renderer-accessibility", "--window-position=$x,$y", "--window-size=1100,1000", $url | Out-Null
    $h = Wait-Until { ([Drv]::Find(0, $null, "Ledge Grab Scroll to PDF"))[0] } 15000 "fixture window"
    Start-Sleep -Milliseconds 1500
    return $h
}

function UIA-Find($h, [string]$name) {
    $root = [Windows.Automation.AutomationElement]::FromHandle($h)
    $cond = New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::NameProperty), $name
    try { Wait-Until { $root.FindFirst([Windows.Automation.TreeScope]::Descendants, $cond) } 15000 "UIA '$name'" }
    catch {
        $names = $root.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition) |
            ForEach-Object { $_.Current.Name } | Where-Object { $_ } | Select-Object -First 25
        Log ("UIA names under the window: " + ($names -join " | "))
        throw
    }
}

# Select the pane (inside its border, left of the scrollbar), and leave the mouse there.
function Select-Pane($sensor, $pane) {
    $r = $pane.Current.BoundingRectangle
    [void][Drv]::PostMessage($sensor, $RUN, [IntPtr]$SCROLL_PDF, [IntPtr]0)
    Wait-Until { ([Drv]::Find(0, "LedgeGrabOverlay", $null))[0] } 5000 "picker overlay" | Out-Null
    Start-Sleep -Milliseconds 500
    $l = [int]$r.Left + 4; $t = [int]$r.Top + 4; $rr = [int]$r.Right - 24; $b = [int]$r.Bottom - 4
    [Drv]::Drag($l, $t, $rr, $b, 20)
    return @($l, $t, $rr, $b)
}

function Wait-Pdf($data, [int]$ms = 90000) {
    try {
        Wait-Until { $log = Preview-Log $data; if ($log -match "Scroll PDF saved|Scroll PDF: |Scroll PDF did not start") { $log } } $ms "Scroll PDF result"
    } catch { Preview-Log $data }
}

function Pdfs($data) { @(Get-ChildItem -Recurse (Join-Path $data "shots") -Filter *.pdf -ErrorAction SilentlyContinue) }

function Run-Capture([string]$name, [string]$file, [scriptblock]$during = $null, [string]$start = "Start at message 06") {
    $data = Join-Path $Out "data-$name"
    $pv = Start-Preview $data
    Ok-Dialog $pv.Dialog
    $sensor = Wait-Until { Sensor $pv.P } 5000 "preview ledge"
    $h = Start-Fixture $file
    $r = [Drv]::Rect($h)
    [Drv]::Click($r.L + 200, $r.T + 140)    # focus the page
    Start-Sleep -Milliseconds 300
    if ($start) {
        $btn = UIA-Find $h $start
        $btn.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
        Start-Sleep -Milliseconds 400
    }
    $pane = UIA-Find $h $(if ($file -like "*repeat*") { "Repeating conversation" } else { "Test conversation" })
    $sel = Select-Pane $sensor $pane
    Log ("$name selection " + ($sel -join ","))
    if ($during) { & $during $h $pane }
    $log = Wait-Pdf $data
    Start-Sleep -Milliseconds 800
    $notch = [Drv]::Rect($sensor)
    [Drv]::Capture($notch.L - 40, 0, $notch.R + 40, $notch.B + 40, (Join-Path $Out "$name-ledge.png"))
    $files = Pdfs $data
    foreach ($f in $files) { Copy-Item $f.FullName (Join-Path $Out ("$name - " + $f.Name)) }
    Copy-Item (Join-Path $data "preview-launch.log") (Join-Path $Out "$name-launch.log") -ErrorAction SilentlyContinue
    [void][Drv]::PostMessage($h, $WM.CLOSE, [IntPtr]0, [IntPtr]0)
    $pv.P.Kill()
    $ended = ([regex]::Match($log, "Scroll PDF ended: [^\r\n]*")).Value
    return @{ Log = $log; Files = $files; Ended = $ended }
}

function Scenario-Fixture06 {
    $r = Run-Capture "fixture06" "scroll_pdf_fixture.html"
    $full = @($r.Files | Where-Object { $_.Name -notmatch "partial" })
    Result "fixture from message 06 saves a complete PDF" ($full.Count -eq 1 -and $r.Ended -match "NoMoreMovement") ("$($r.Ended); files: " + (($r.Files | ForEach-Object Name) -join ", "))
}

function Scenario-Esc {
    $r = Run-Capture "esc" "scroll_pdf_fixture.html" { param($h, $pane) Start-Sleep -Milliseconds 2600; [Drv]::Tap($VK.ESC, $false); Start-Sleep -Milliseconds 30; [Drv]::Tap($VK.ESC, $true) } "Reset to top"
    $partial = @($r.Files | Where-Object { $_.Name -match "partial" })
    Result "Esc mid-run saves a PDF labelled partial" ($partial.Count -eq 1 -and $r.Ended -match "UserStopped") ("$($r.Ended); files: " + (($r.Files | ForEach-Object Name) -join ", "))
}

function Scenario-Covered {
    $r = Run-Capture "covered" "scroll_pdf_fixture.html" {
        param($h, $pane)
        Start-Sleep -Milliseconds 2200
        $b = $pane.Current.BoundingRectangle
        $c = New-Object Cover ([int]$b.Left + 60), ([int]$b.Top + 60), 220, 120
        $c.Show()
        $end = (Get-Date).AddSeconds(4)
        while ((Get-Date) -lt $end) { [Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 30 }
        $c.Close()
    } "Reset to top"
    Result "covering the pane stops with a partial PDF, not a guess" (@($r.Files | Where-Object { $_.Name -match "partial" }).Count -eq 1 -and $r.Ended -match "PaneCovered") ("$($r.Ended); files: " + (($r.Files | ForEach-Object Name) -join ", "))
}

function Scenario-Moving {
    $r = Run-Capture "moving" "scroll_pdf_fixture.html" { param($h, $pane) Start-Sleep -Milliseconds 2200; [Drv]::MoveBy($h, 0, 70) } "Reset to top"
    $partial = @($r.Files | Where-Object { $_.Name -match "partial" })
    Result "moving the window stops with a partial PDF" ($partial.Count -eq 1 -and $r.Ended -match "TargetChanged") ("$($r.Ended); files: " + (($r.Files | ForEach-Object Name) -join ", "))
}

function Scenario-Seam {
    $r = Run-Capture "seam" "scroll_pdf_repeat.html" $null $null
    Result "identical rows: uncertain seam stops and labels partial" (@($r.Files | Where-Object { $_.Name -match "partial" }).Count -eq 1 -and $r.Ended -match "NoSafeOverlap") ("$($r.Ended); files: " + (($r.Files | ForEach-Object Name) -join ", "))
}

$all = "StartupClean", "StartupWhileRunning", "Fixture06", "Esc", "Covered", "Moving", "Seam"
foreach ($s in $all) {
    if ($Only.Count -and $Only -notcontains $s) { continue }
    Log "== $s"
    try { & "Scenario-$s" } catch { Result $s $false ("error: " + $_.Exception.Message + " @ " + $_.InvocationInfo.ScriptLineNumber) }
    Get-Process ledge-grab -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 800
}
Stop-TestEdge
Log "== SUMMARY"
$results | ForEach-Object { Log $_ }
Set-Content -Path (Join-Path $Out "done.flag") -Value ($results -join "`n")
