param([switch]$Debug, [switch]$Screenshot)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot
try {
    $localCargo = Join-Path $projectRoot '.cache\cargo\bin\cargo.exe'
    if (Test-Path -LiteralPath $localCargo) {
        $env:CARGO_HOME = Join-Path $projectRoot '.cache\cargo'
        $env:RUSTUP_HOME = Join-Path $projectRoot '.cache\rustup'
        $env:PATH = "$(Split-Path -Parent $localCargo);$env:PATH"
        $nativeTools = Join-Path $projectRoot '.cache\build-tools\w64devkit\bin'
        if (Test-Path -LiteralPath $nativeTools) {
            $env:PATH = "$nativeTools;$env:PATH"
            $env:LIBRARY_PATH = Join-Path $env:RUSTUP_HOME 'toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\lib\self-contained'
        }
    }
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargo) { throw 'Install Rust from https://rustup.rs, then run this script again.' }
    $buildArgs = @('build', '--locked')
    if (-not $Debug) { $buildArgs += '--release' }
    if ($Screenshot) { $buildArgs += @('--features', 'screenshot') }
    & $cargo @buildArgs
    if ($LASTEXITCODE -ne 0) { throw "Rust build failed ($LASTEXITCODE)" }
    $profile = if ($Debug) { 'debug' } else { 'release' }
    Copy-Item -LiteralPath (Join-Path $projectRoot "target\$profile\pixclip.exe") -Destination (Join-Path $projectRoot 'PixClip.exe')
    Write-Output 'Built PixClip.exe. Run .\PixClip.bat or .\PixClip.exe --cli.'
} finally { Pop-Location }
