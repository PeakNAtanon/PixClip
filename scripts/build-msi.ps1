param(
    [string]$Executable,
    [string]$WixCommand = 'wix',
    [string]$UiExtension = 'WixToolset.UI.wixext'
)

$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$manifest = Get-Content -Raw (Join-Path $project 'Cargo.toml')
if ($manifest -notmatch '(?m)^version = "(\d+\.\d+\.\d+)"') {
    throw 'Cargo.toml needs a three-part Windows Installer version.'
}
$version = $Matches[1]
if (!$Executable) {
    $Executable = Join-Path $project 'target/release/pixclip.exe'
}
$binary = (Resolve-Path -LiteralPath $Executable).Path
$dist = Join-Path $project 'dist'
$work = Join-Path $project '.cache/msi'
New-Item -ItemType Directory -Path $dist, $work -Force | Out-Null

$license = Get-Content -Raw (Join-Path $project 'LICENSE')
$escaped = $license.Replace('\', '\\').Replace('{', '\{').Replace('}', '\}')
$rtf = '{\rtf1\ansi\deff0{\fonttbl{\f0 Segoe UI;}}\f0\fs20 ' +
    ($escaped -replace '\r?\n', '\par ') + '}'
$licenseRtf = Join-Path $work 'license.rtf'
[IO.File]::WriteAllText($licenseRtf, $rtf, [Text.Encoding]::ASCII)

$output = Join-Path $dist "PixClip-$version-windows-x64.msi"
$wixArgs = @(
    'build', '-arch', 'x64', '-ext', $UiExtension, '-pdbtype', 'none',
    '-d', "Version=$version", '-d', "SourceDir=$project",
    '-d', "Executable=$binary", '-d', "LicenseRtf=$licenseRtf",
    '-o', $output, (Join-Path $PSScriptRoot 'windows/PixClip.wxs')
)
& $WixCommand @wixArgs
if ($LASTEXITCODE -ne 0) {
    throw "WiX build failed with exit code $LASTEXITCODE."
}
Write-Output $output
