# Desktop helpers for Windows smoke tests. Dot-source from a script that runs in the
# signed-in user's interactive session (not over SSH's service session).
# Ledge Grab is driven with posted messages like tests/drive.py; target apps get
# real SendInput keystrokes, because the paste hook only sees real input.
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -ReferencedAssemblies System.Drawing, System.Windows.Forms -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class Drv {
    [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr v);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort vk; public ushort scan; public uint flags; public uint time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx; public int dy; public uint data; public uint flags; public uint time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit)] struct UNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public UNION u; }
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] i, int size);

    public static void DpiAware() { SetProcessDpiAwarenessContext(new IntPtr(-4)); }

    public static IntPtr[] Find(uint pid, string cls, string titlePrefix) {
        var found = new List<IntPtr>();
        EnumWindows(delegate (IntPtr h, IntPtr l) {
            uint p; GetWindowThreadProcessId(h, out p);
            var c = new StringBuilder(256); GetClassName(h, c, 256);
            var t = new StringBuilder(512); GetWindowText(h, t, 512);
            if ((pid == 0 || p == pid) && (string.IsNullOrEmpty(cls) || c.ToString() == cls) && (string.IsNullOrEmpty(titlePrefix) || t.ToString().StartsWith(titlePrefix)) && IsWindowVisible(h))
                found.Add(h);
            return true;
        }, IntPtr.Zero);
        return found.ToArray();
    }

    public static string Title(IntPtr h) { var t = new StringBuilder(512); GetWindowText(h, t, 512); return t.ToString(); }
    public static RECT Rect(IntPtr h) { RECT r; GetWindowRect(h, out r); return r; }

    static INPUT Key(ushort vk, bool up) {
        var i = new INPUT(); i.type = 1; i.u.ki.vk = vk; i.u.ki.flags = up ? 2u : 0u; return i;
    }
    public static void Keys(ushort[] vks, bool[] ups) {
        var a = new INPUT[vks.Length];
        for (int n = 0; n < vks.Length; n++) a[n] = Key(vks[n], ups[n]);
        SendInput((uint)a.Length, a, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void Tap(ushort vk, bool up) { Keys(new ushort[] { vk }, new bool[] { up }); }

    public static void Click(int x, int y) {
        SetCursorPos(x, y);
        System.Threading.Thread.Sleep(40);
        var a = new INPUT[2];
        a[0].type = 0; a[0].u.mi.flags = 0x0002;
        a[1].type = 0; a[1].u.mi.flags = 0x0004;
        SendInput(2, a, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void Drag(int x0, int y0, int x1, int y1, int steps) {
        SetCursorPos(x0, y0);
        System.Threading.Thread.Sleep(60);
        var d = new INPUT[1]; d[0].type = 0; d[0].u.mi.flags = 0x0002;
        SendInput(1, d, Marshal.SizeOf(typeof(INPUT)));
        for (int n = 1; n <= steps; n++) {
            SetCursorPos(x0 + (x1 - x0) * n / steps, y0 + (y1 - y0) * n / steps);
            System.Threading.Thread.Sleep(15);
        }
        System.Threading.Thread.Sleep(80);
        var u = new INPUT[1]; u[0].type = 0; u[0].u.mi.flags = 0x0004;
        SendInput(1, u, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void Park(int x, int y) { SetCursorPos(x, y); }

    delegate bool ChildProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, ChildProc f, IntPtr l);
    /// Text of a message box: its static controls joined.
    public static string DialogText(IntPtr dlg) {
        var sb = new StringBuilder();
        EnumChildWindows(dlg, delegate (IntPtr h, IntPtr l) {
            var c = new StringBuilder(64); GetClassName(h, c, 64);
            if (c.ToString() == "Static") { var t = new StringBuilder(2048); GetWindowText(h, t, 2048); if (t.Length > 0) sb.Append(t.ToString()).Append(" | "); }
            return true;
        }, IntPtr.Zero);
        return sb.ToString();
    }
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int hh, uint flags);
    public static void MoveBy(IntPtr h, int dx, int dy) {
        var r = Rect(h);
        SetWindowPos(h, IntPtr.Zero, r.L + dx, r.T + dy, 0, 0, 0x0001 | 0x0004 | 0x0010);
    }
    public static void Place(IntPtr h, int x, int y, int w, int hh) { SetWindowPos(h, IntPtr.Zero, x, y, w, hh, 0x0004 | 0x0010); }

    public static void Capture(int l, int t, int r, int b, string path) {
        using (var bmp = new System.Drawing.Bitmap(r - l, b - t))
        using (var g = System.Drawing.Graphics.FromImage(bmp)) {
            g.CopyFromScreen(l, t, 0, 0, bmp.Size);
            bmp.Save(path, System.Drawing.Imaging.ImageFormat.Png);
        }
    }
}

/// A topmost window that never takes focus, to cover part of the screen.
public class Cover : System.Windows.Forms.Form {
    public Cover(int x, int y, int w, int h) {
        FormBorderStyle = System.Windows.Forms.FormBorderStyle.None;
        StartPosition = System.Windows.Forms.FormStartPosition.Manual;
        Bounds = new System.Drawing.Rectangle(x, y, w, h);
        BackColor = System.Drawing.Color.DarkOrange;
        TopMost = true;
        ShowInTaskbar = false;
    }
    protected override bool ShowWithoutActivation { get { return true; } }
    protected override System.Windows.Forms.CreateParams CreateParams {
        get { var p = base.CreateParams; p.ExStyle |= 0x08000000 | 0x00000008; return p; }
    }
}
"@
[Drv]::DpiAware()

$VK = @{ CTRL = 0x11; SHIFT = 0x10; V = 0x56; Q = 0x51; ESC = 0x1B; A = 0x41 }
$WM = @{ MOVE = 0x200; DOWN = 0x201; UP = 0x202; WHEEL = 0x20A; KEYDOWN = 0x100; CLOSE = 0x10 }

function Log($m) { $line = "{0:HH:mm:ss.fff} {1}" -f (Get-Date), $m; Write-Output $line; Add-Content -Path $global:LogFile -Value $line }

function Wait-Until([scriptblock]$check, [int]$ms = 5000, [string]$what = "condition") {
    $end = (Get-Date).AddMilliseconds($ms)
    while ((Get-Date) -lt $end) { $v = & $check; if ($v) { return $v }; Start-Sleep -Milliseconds 50 }
    throw "Timed out waiting for $what"
}

# Ledge Grab in its isolated test mode (separate data dir, no hotkeys).
function Start-Ledge([string]$exe, [string]$data, [hashtable]$extraEnv = @{}) {
    Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $data | Out-Null
    $psi = New-Object Diagnostics.ProcessStartInfo $exe
    $psi.UseShellExecute = $false
    $psi.EnvironmentVariables["LEDGE_GRAB_TEST"] = "1"
    $psi.EnvironmentVariables["LEDGE_GRAB_DIR"] = $data
    $psi.EnvironmentVariables["LEDGE_GRAB_LOG"] = (Join-Path $data "log.txt")
    foreach ($k in $extraEnv.Keys) { $psi.EnvironmentVariables[$k] = $extraEnv[$k] }
    $p = [Diagnostics.Process]::Start($psi)
    $h = Wait-Until { ([Drv]::Find([uint32]$p.Id, "LedgeGrabNotch", "Ledge Grab sensor"))[0] } 8000 "ledge sensor window"
    $view = ([Drv]::Find([uint32]$p.Id, "LedgeGrabNotch", "Ledge Grab"))[0]
    Start-Sleep -Milliseconds 900
    $s = [Drv]::GetDpiForWindow($h) / 96.0
    return @{ P = $p; H = $h; View = $view; S = $s; Data = $data }
}

function Make-LParam([double]$x, [double]$y) {
    $xi = [int64][math]::Round($x) -band 0xFFFF
    $yi = [int64][math]::Round($y) -band 0xFFFF
    return [IntPtr](($yi * 65536) + $xi)
}
function Post($ledge, $msg, $x = 0, $y = 0, $wp = 0) {
    $lp = Make-LParam -x ($x * $ledge.S) -y ($y * $ledge.S)
    [void][Drv]::PostMessage($ledge.H, [uint32]$msg, [IntPtr][int64]$wp, $lp)
}
function Ledge-Click($ledge, $x, $y) {
    Post $ledge $WM.MOVE $x $y; Start-Sleep -Milliseconds 50
    Post $ledge $WM.DOWN $x $y; Start-Sleep -Milliseconds 40
    Post $ledge $WM.UP $x $y; Start-Sleep -Milliseconds 350
}
function Ledge-Size($ledge) { $r = [Drv]::Rect($ledge.H); return @([math]::Round(($r.R - $r.L) / $ledge.S), [math]::Round(($r.B - $r.T) / $ledge.S)) }
function Ledge-Shot($ledge, $path, $pad = 30) {
    $r = [Drv]::Rect($ledge.H)
    [Drv]::Capture($r.L - $pad, [math]::Max($r.T, 0), $r.R + $pad, $r.B + $pad, $path)
}
function Stop-Ledge($ledge) {
    [void][Drv]::PostMessage($ledge.H, $WM.CLOSE, [IntPtr]0, [IntPtr]0)
    if (-not $ledge.P.WaitForExit(3000)) { $ledge.P.Kill() }
}

# A solid image with a big letter, put on the clipboard like another app would.
function Copy-Solid([System.Drawing.Color]$c, [string]$letter) {
    $b = New-Object Drawing.Bitmap 240, 150
    $g = [Drawing.Graphics]::FromImage($b)
    $g.Clear($c)
    $f = New-Object Drawing.Font "Segoe UI", 60, ([Drawing.FontStyle]::Bold)
    $g.DrawString($letter, $f, [Drawing.Brushes]::White, 80, 20)
    $g.Dispose()
    [Windows.Forms.Clipboard]::SetImage($b)
    $b.Dispose()
}

function Paste-Chord([int]$times, [int]$gapMs) {
    [Drv]::Tap($VK.CTRL, $false)
    for ($n = 0; $n -lt $times; $n++) {
        [Drv]::Tap($VK.V, $false); Start-Sleep -Milliseconds 15; [Drv]::Tap($VK.V, $true)
        if ($n -lt $times - 1) { Start-Sleep -Milliseconds $gapMs }
    }
    Start-Sleep -Milliseconds 20
    [Drv]::Tap($VK.CTRL, $true)
}

# Close every Edge started with a throwaway lg-edge-* profile and delete those profiles.
function Stop-TestEdge {
    Get-CimInstance Win32_Process | Where-Object { $_.Name -eq "msedge.exe" -and $_.CommandLine -like "*lg-edge-*" } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 800
    Get-ChildItem $env:TEMP -Directory -Filter "lg-edge-*" | ForEach-Object { Remove-Item -LiteralPath $_.FullName -Recurse -Force -ErrorAction SilentlyContinue }
}
