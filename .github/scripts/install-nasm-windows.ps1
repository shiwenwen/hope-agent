# Install NASM on Windows CI runners.
#
# Vendored OpenSSL (pulled in transitively on Windows MSVC) needs NASM at build
# time. We install through Chocolatey, which is pre-installed on the GitHub
# windows-latest image. The community Chocolatey CDN occasionally returns HTTP
# 499 on the V2 feed, so we retry with exponential backoff and, as a last
# resort, fall back to NASM's official Windows binary archive before failing.

$ErrorActionPreference = 'Continue'

function Test-NasmInstalled {
    return Test-Path 'C:\Program Files\NASM\nasm.exe'
}

$attempts = 4
$installed = $false
for ($i = 1; $i -le $attempts; $i++) {
    Write-Host "::group::choco install nasm (attempt $i/$attempts)"
    choco install nasm -y --no-progress
    $code = $LASTEXITCODE
    Write-Host "::endgroup::"
    if ($code -eq 0 -and (Test-NasmInstalled)) {
        $installed = $true
        break
    }
    if ($i -lt $attempts) {
        $delay = [int]([Math]::Pow(2, $i) * 5)
        Write-Host "choco install failed (exit $code); retrying in $delay s"
        Start-Sleep -Seconds $delay
    } else {
        Write-Host "choco install failed after $attempts attempts (last exit $code); falling back to direct download"
    }
}

if (-not $installed) {
    $version = '2.16.03'
    $url = "https://www.nasm.us/pub/nasm/releasebuilds/$version/win64/nasm-$version-win64.zip"
    $zip = Join-Path $env:RUNNER_TEMP 'nasm.zip'
    $extractDir = Join-Path $env:RUNNER_TEMP 'nasm-extract'
    $dest = 'C:\Program Files\NASM'

    $downloadAttempts = 3
    for ($i = 1; $i -le $downloadAttempts; $i++) {
        Write-Host "::group::download NASM $version (attempt $i/$downloadAttempts)"
        try {
            Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing -ErrorAction Stop
            Write-Host "::endgroup::"
            break
        } catch {
            Write-Host "download failed: $_"
            Write-Host "::endgroup::"
            if ($i -eq $downloadAttempts) {
                throw "NASM fallback download failed after $downloadAttempts attempts"
            }
            Start-Sleep -Seconds ([int]([Math]::Pow(2, $i) * 5))
        }
    }

    if (Test-Path $extractDir) { Remove-Item $extractDir -Recurse -Force }
    Expand-Archive -Path $zip -DestinationPath $extractDir -Force

    $srcRoot = Get-ChildItem -Directory $extractDir | Select-Object -First 1
    if (-not $srcRoot) { throw "extracted NASM archive is empty" }

    if (-not (Test-Path $dest)) { New-Item -ItemType Directory -Path $dest -Force | Out-Null }
    Copy-Item -Path (Join-Path $srcRoot.FullName '*') -Destination $dest -Recurse -Force

    if (-not (Test-NasmInstalled)) {
        throw "NASM fallback install completed but nasm.exe is missing at $dest"
    }
}

"C:\Program Files\NASM" | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
& 'C:\Program Files\NASM\nasm.exe' -v
