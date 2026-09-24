# Generates the Aura icon set from the geometric "A" path (same shape
# as assets/icon/aura.svg). Pure System.Drawing — no external tools.
#
#   pwsh scripts/make-icons.ps1
#
# Outputs into assets/icon/: aura.ico (16/32/48/256, PNG-in-ICO),
# icon.png (512), favicon.png (32), social.png (1280x640 banner).
param()

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$outDir = Join-Path $root 'assets/icon'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

Add-Type -AssemblyName System.Drawing

# The "A" silhouette, scaled to a unit box then mapped to canvas.
# Outer polygon (clockwise): apex flat, right leg, crossbar shelf,
# left leg. Inner polygon: the counter triangle.
$script:outer = @(
    @(116,24), @(140,24), @(224,232), @(180,232),
    @(164,188), @(92,188), @(76,232), @(32,232)
)
$script:inner = @( @(128,88), @(156,148), @(100,148) )

function New-AuraBitmap([int]$size) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.Clear([System.Drawing.Color]::Transparent)
    $scale = $size / 256.0
    $toPt = { param($p) New-Object System.Drawing.PointF ($p[0]*$scale), ($p[1]*$scale) }
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $path.FillMode = 'Alternate'
    $path.AddPolygon([System.Drawing.PointF[]]($script:outer | ForEach-Object { & $toPt $_ }))
    $path.AddPolygon([System.Drawing.PointF[]]($script:inner | ForEach-Object { & $toPt $_ }))
    $g.FillPath([System.Drawing.Brushes]::Black, $path)
    $g.Dispose()
    return $bmp
}

function Save-Png($bmp, [string]$path) {
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
}

# --- individual PNGs ---------------------------------------------------------
$pngBytes = @{}
foreach ($s in 16, 32, 48, 256) {
    $b = New-AuraBitmap $s
    $ms = New-Object System.IO.MemoryStream
    $b.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $pngBytes[$s] = $ms.ToArray()
    $b.Dispose(); $ms.Dispose()
}
$b = New-AuraBitmap 512; Save-Png $b (Join-Path $outDir 'icon.png'); $b.Dispose()
$b = New-AuraBitmap 32;  Save-Png $b (Join-Path $outDir 'favicon.png'); $b.Dispose()
$b = New-AuraBitmap 256; Save-Png $b (Join-Path $outDir 'aura-256.png'); $b.Dispose()

# --- social banner 1280x640: icon + "AURA" -----------------------------------
$banner = New-Object System.Drawing.Bitmap 1280, 640
$g = [System.Drawing.Graphics]::FromImage($banner)
$g.SmoothingMode = 'AntiAlias'
$g.TextRenderingHint = 'AntiAlias'
$g.Clear([System.Drawing.Color]::White)
$mark = New-AuraBitmap 320
$g.DrawImage($mark, 140, 160, 320, 320)
$font = New-Object System.Drawing.Font 'Segoe UI', 120, ([System.Drawing.FontStyle]::Bold)
$g.DrawString('AURA', $font, [System.Drawing.Brushes]::Black, 500, 220)
$banner.Save((Join-Path $outDir 'social.png'), [System.Drawing.Imaging.ImageFormat]::Png)
$font.Dispose(); $mark.Dispose(); $g.Dispose(); $banner.Dispose()

# --- aura.ico: PNG-in-ICO (valid since Vista) --------------------------------
$sizes = @(16, 32, 48, 256)
$ms = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter $ms
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
foreach ($s in $sizes) {
    $data = $pngBytes[$s]
    $w.Write([byte]($s -band 0xFF))          # width (256 -> 0)
    $w.Write([byte]($s -band 0xFF))          # height
    $w.Write([byte]0)                        # palette
    $w.Write([byte]0)                        # reserved
    $w.Write([uint16]1)                      # planes
    $w.Write([uint16]32)                     # bpp
    $w.Write([uint32]$data.Length)
    $w.Write([uint32]$offset)
    $offset += $data.Length
}
foreach ($s in $sizes) { $w.Write($pngBytes[$s]) }
$w.Flush()
[System.IO.File]::WriteAllBytes((Join-Path $outDir 'aura.ico'), $ms.ToArray())
$w.Dispose(); $ms.Dispose()

# --- wizard.bmp: 55x58 for Inno's WizardSmallImageFile -------------------------
$wb = New-Object System.Drawing.Bitmap 55, 58
$g = [System.Drawing.Graphics]::FromImage($wb)
$g.SmoothingMode = 'AntiAlias'
$g.Clear([System.Drawing.Color]::White)
$mark = New-AuraBitmap 48
$g.DrawImage($mark, 4, 5, 48, 48)
$wb.Save((Join-Path $outDir 'wizard.bmp'), [System.Drawing.Imaging.ImageFormat]::Bmp)
$mark.Dispose(); $g.Dispose(); $wb.Dispose()

Get-ChildItem $outDir | ForEach-Object { "{0,10} {1}" -f $_.Length, $_.Name }
