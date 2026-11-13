#Requires -Version 7
# Draws the app icon: a red rounded square (the tray "V" colour) with "Gõ" in white Segoe UI Bold.
# Writes app/govn.ico (embedded in govn.exe, also the settings window icon) and docs/icon.png.
# Usage: pwsh -NoProfile -File tools/make_icon.ps1
Add-Type -AssemblyName System.Drawing
$root = Split-Path -Parent $PSScriptRoot

function Draw-Icon([int]$size) {
    $bmp = [System.Drawing.Bitmap]::new($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.PixelOffsetMode = 'HighQuality'
    $g.Clear([System.Drawing.Color]::Transparent)

    # Rounded square with a gentle top-to-bottom gradient.
    $inset = [math]::Max(0, [math]::Round($size / 32))
    $box = [System.Drawing.RectangleF]::new($inset, $inset, $size - 2 * $inset, $size - 2 * $inset)
    $r = $box.Width * 0.22
    $path = [System.Drawing.Drawing2D.GraphicsPath]::new()
    $path.AddArc($box.X, $box.Y, 2 * $r, 2 * $r, 180, 90)
    $path.AddArc($box.Right - 2 * $r, $box.Y, 2 * $r, 2 * $r, 270, 90)
    $path.AddArc($box.Right - 2 * $r, $box.Bottom - 2 * $r, 2 * $r, 2 * $r, 0, 90)
    $path.AddArc($box.X, $box.Bottom - 2 * $r, 2 * $r, 2 * $r, 90, 90)
    $path.CloseFigure()
    $top = [System.Drawing.Color]::FromArgb(255, 0xE5, 0x45, 0x3F)
    $bottom = [System.Drawing.Color]::FromArgb(255, 0xC2, 0x27, 0x27)
    $fill = [System.Drawing.Drawing2D.LinearGradientBrush]::new($box, $top, $bottom, 90)
    $g.FillPath($fill, $path)

    # "Gõ" from the name, as a filled outline centred on its real ink bounds (not the font's line box).
    $w = $box.Width
    $text = [System.Drawing.Drawing2D.GraphicsPath]::new()
    $family = [System.Drawing.FontFamily]::new('Segoe UI')
    $text.AddString('Gõ', $family, [int][System.Drawing.FontStyle]::Bold, 100, [System.Drawing.PointF]::new(0, 0), [System.Drawing.StringFormat]::GenericTypographic)
    $ink = $text.GetBounds()
    # Small sizes have no room for margins: let the letters fill more of the square.
    $fit = $(if ($size -le 24) { 0.86 } else { 0.70 })
    $scale = [math]::Min($w * $fit / $ink.Width, $w * $fit / $ink.Height)
    $m = [System.Drawing.Drawing2D.Matrix]::new()
    $m.Translate($box.X + ($w - $ink.Width * $scale) / 2, $box.Y + ($w - $ink.Height * $scale) / 2)
    $m.Scale($scale, $scale)
    $m.Translate(-$ink.X, -$ink.Y)
    $text.Transform($m)
    $g.FillPath([System.Drawing.Brushes]::White, $text)
    $g.Dispose()
    return $bmp
}

function Png-Bytes($bmp) {
    $ms = [System.IO.MemoryStream]::new()
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    return , $ms.ToArray()
}

# ICO with PNG entries (Windows Vista and later read these at every size).
$sizes = 16, 20, 24, 32, 40, 48, 64, 256
$images = foreach ($s in $sizes) { , (Png-Bytes (Draw-Icon $s)) }
$out = [System.IO.MemoryStream]::new()
$bw = [System.IO.BinaryWriter]::new($out)
$bw.Write([uint16]0); $bw.Write([uint16]1); $bw.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]
    $bw.Write([byte]($s % 256)); $bw.Write([byte]($s % 256))
    $bw.Write([byte]0); $bw.Write([byte]0)
    $bw.Write([uint16]1); $bw.Write([uint16]32)
    $bw.Write([uint32]$images[$i].Length); $bw.Write([uint32]$offset)
    $offset += $images[$i].Length
}
foreach ($img in $images) { $bw.Write($img) }
$bw.Flush()
[IO.File]::WriteAllBytes((Join-Path $root 'app/govn.ico'), $out.ToArray())
(Draw-Icon 256).Save((Join-Path $root 'docs/icon.png'), [System.Drawing.Imaging.ImageFormat]::Png)
"wrote app/govn.ico ($($sizes -join ', ') px), docs/icon.png"
