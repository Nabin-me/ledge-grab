# A second paste target that reads images the classic way (Clipboard.GetImage,
# i.e. CF_BITMAP/CF_DIB through OLE). Each Ctrl+V records the image's colour in
# the window title: red = A, green = B, blue = C.
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$f = New-Object Windows.Forms.Form
$f.Text = "WINFORMS TARGET READY"
$f.Width = 700; $f.Height = 400; $f.StartPosition = "Manual"; $f.Left = 200; $f.Top = 300
$f.KeyPreview = $true
$seen = New-Object Collections.ArrayList
$f.Add_KeyDown({
    param($s, $e)
    if ($e.Control -and $e.KeyCode -eq [Windows.Forms.Keys]::V) {
        $img = [Windows.Forms.Clipboard]::GetImage()
        if ($img -eq $null) { [void]$seen.Add("none") } else {
            $p = ([Drawing.Bitmap]$img).GetPixel(4, 4)
            $n = if ($p.R -gt 200 -and $p.G -lt 80 -and $p.B -lt 80) { "A" } elseif ($p.G -gt 200 -and $p.R -lt 80 -and $p.B -lt 80) { "B" } elseif ($p.B -gt 200 -and $p.R -lt 80 -and $p.G -lt 80) { "C" } else { "?" }
            [void]$seen.Add($n); $img.Dispose()
        }
        $f.Text = "PASTES:" + ($seen -join ",")
        $e.Handled = $true
    }
})
[Windows.Forms.Application]::Run($f)
