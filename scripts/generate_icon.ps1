# ==============================================================================
# Generate Multi-Resolution Windows .ico and PNG assets from master logo
# ==============================================================================

Add-Type -AssemblyName System.Drawing

$sourcePath = "C:\Users\Rushikesh\.gemini\antigravity-ide\brain\9e811f3a-9b4b-4e83-a52c-21bdc4d0de63\nexus_logo_1792823472954.jpg"
$assetsDir = "d:\Data\DATA\projects\personal\compression\assets"

if (-not (Test-Path $assetsDir)) {
    New-Item -ItemType Directory -Path $assetsDir -Force | Out-Null
}

$pngPath = Join-Path $assetsDir "nexus_logo.png"
$icoPath = Join-Path $assetsDir "nexus.ico"

# 1. Load source image
$srcImg = [System.Drawing.Image]::FromFile($sourcePath)

# 2. Save 256x256 PNG
$sizes = @(256, 48, 32, 16)
$pngStreams = @()

foreach ($sz in $sizes) {
    $bmp = New-Object System.Drawing.Bitmap($sz, $sz, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.DrawImage($srcImg, 0, 0, $sz, $sz)
    $g.Dispose()

    if ($sz -eq 256) {
        $bmp.Save($pngPath, [System.Drawing.Imaging.ImageFormat]::Png)
    }

    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    $pngStreams += ,@($sz, $ms.ToArray())
    $ms.Dispose()
}

$srcImg.Dispose()

# 3. Assemble Windows ICO
$icoStream = New-Object System.IO.MemoryStream
$writer = New-Object System.IO.BinaryWriter($icoStream)

# ICONDIR Header
$writer.Write([uint16]0) # Reserved
$writer.Write([uint16]1) # Type: 1 = Icon
$writer.Write([uint16]$pngStreams.Count) # Image count

# Calculate initial offset for image data: 6 + (16 * count)
$offset = 6 + (16 * $pngStreams.Count)

foreach ($item in $pngStreams) {
    $sz = $item[0]
    $data = $item[1]

    $wByte = if ($sz -ge 256) { [byte]0 } else { [byte]$sz }
    $hByte = if ($sz -ge 256) { [byte]0 } else { [byte]$sz }

    $writer.Write($wByte)          # Width
    $writer.Write($hByte)          # Height
    $writer.Write([byte]0)         # ColorCount
    $writer.Write([byte]0)         # Reserved
    $writer.Write([uint16]1)       # Planes
    $writer.Write([uint16]32)      # BitCount
    $writer.Write([uint32]$data.Length) # BytesInRes
    $writer.Write([uint32]$offset) # ImageOffset

    $offset += $data.Length
}

# Write image data
foreach ($item in $pngStreams) {
    $data = $item[1]
    $writer.Write($data)
}

[System.IO.File]::WriteAllBytes($icoPath, $icoStream.ToArray())
$writer.Dispose()
$icoStream.Dispose()

Write-Host "Generated assets successfully:"
Write-Host " - PNG: $pngPath"
Write-Host " - ICO: $icoPath (Size: $((Get-Item $icoPath).Length) bytes)"
