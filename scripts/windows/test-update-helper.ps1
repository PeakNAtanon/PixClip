$ErrorActionPreference = 'Stop'
$global:PixClipUpdateTest = [pscustomobject]@{CaseName='';InstallDirectory='';MsiPath='';InstallerCalls=0;Restarts=@();UpdateError=$null}
$project = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureRoot = Join-Path $project '.cache/update-helper-checks'
New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null
$source = Get-Content -Raw -Encoding UTF8 (Join-Path $project 'scripts/windows/install-update.ps1')
$displayCall = '[void][System.Windows.Forms.MessageBox]::Show($_.Exception.Message, ''PixClip update'', ''OK'', ''Error'')'
if (-not $source.Contains($displayCall)) { throw 'Expected notification call was not found' }
$source = $source.Replace($displayCall, '$global:PixClipUpdateTest.UpdateError = $_.Exception.Message')
$helperCopy = Join-Path $fixtureRoot 'helper-under-test.ps1'
[IO.File]::WriteAllText($helperCopy, $source, (New-Object Text.UTF8Encoding($false)))
if (-not $env:SystemRoot) { $env:SystemRoot = 'C:\Windows' }

function Get-Process {
    param($Id, $ErrorAction)
    if ($global:PixClipUpdateTest.CaseName -eq 'parent-timeout') {
        $parent = New-Object PSObject
        $parent | Add-Member -MemberType ScriptMethod -Name WaitForExit -Value { param($timeout); return $false }
        return $parent
    }
    return $null
}
function Get-ItemProperty {
    param($LiteralPath, $Name, $ErrorAction)
    if ($LiteralPath -ne 'HKCU:\Software\PixClip\Installer') { throw 'Unexpected registry access' }
    return [pscustomobject]@{InstallFolder=$global:PixClipUpdateTest.InstallDirectory}
}
function Start-Process {
    param($FilePath, $ArgumentList, $WindowStyle, [switch]$Wait, [switch]$PassThru)
    if ($FilePath -like '*msiexec.exe') {
        if (-not $Wait -or -not $PassThru -or $WindowStyle -ne 'Hidden') { throw 'Installer process flags changed' }
        if ($ArgumentList -notcontains '/passive' -or $ArgumentList -notcontains '/norestart') { throw 'Expected installer arguments missing' }
        if ($ArgumentList -notcontains ('"{0}"' -f $global:PixClipUpdateTest.MsiPath)) { throw 'Unicode/space installer argument changed' }
        $global:PixClipUpdateTest.InstallerCalls++
        $code = if ($global:PixClipUpdateTest.CaseName -eq 'installer-failure') { 1603 } elseif ($global:PixClipUpdateTest.CaseName -eq 'reboot-required') { 3010 } else { 0 }
        return [pscustomobject]@{ExitCode=$code}
    }
    $global:PixClipUpdateTest.Restarts += $FilePath
}

foreach ($caseName in @('success','reboot-required','installer-failure','tampered','parent-timeout')) {
    $global:PixClipUpdateTest.CaseName = $caseName
    $folder = Join-Path $fixtureRoot $global:PixClipUpdateTest.CaseName
    New-Item -ItemType Directory -Path $folder -Force | Out-Null
    $global:PixClipUpdateTest.InstallDirectory = Join-Path $folder 'new app'
    New-Item -ItemType Directory -Path $global:PixClipUpdateTest.InstallDirectory -Force | Out-Null
    $newExe = Join-Path $global:PixClipUpdateTest.InstallDirectory 'PixClip.exe'
    $oldExe = Join-Path $folder 'original.exe'
    [IO.File]::WriteAllText($newExe, 'fixture, never executed')
    [IO.File]::WriteAllText($oldExe, 'fixture, never executed')
    $global:PixClipUpdateTest.MsiPath = Join-Path $folder (([char]0x0E01).ToString() + ' update.msi')
    [IO.File]::WriteAllText($global:PixClipUpdateTest.MsiPath, 'fixture installer, never executed')
    $hash = (Get-FileHash -LiteralPath $global:PixClipUpdateTest.MsiPath -Algorithm SHA256).Hash
    if ($global:PixClipUpdateTest.CaseName -eq 'tampered') { $hash = '0' * 64 }
    $plan = [pscustomobject]@{pid=123456;msi=$global:PixClipUpdateTest.MsiPath;hash=$hash;original=$oldExe;log=(Join-Path $folder 'install.log')}
    $planPath = Join-Path $folder 'plan.json'
    [IO.File]::WriteAllText($planPath, ($plan | ConvertTo-Json), (New-Object Text.UTF8Encoding($false)))
    $global:PixClipUpdateTest.InstallerCalls = 0; $global:PixClipUpdateTest.Restarts = @(); $global:PixClipUpdateTest.UpdateError = $null
    & $helperCopy -Plan $planPath
    if ($global:PixClipUpdateTest.CaseName -in @('success','reboot-required')) {
        if ($global:PixClipUpdateTest.InstallerCalls -ne 1 -or $global:PixClipUpdateTest.Restarts.Count -ne 1 -or $global:PixClipUpdateTest.Restarts[0] -ne $newExe -or $global:PixClipUpdateTest.UpdateError) { throw ("Failed " + $caseName) }
    } elseif ($global:PixClipUpdateTest.CaseName -eq 'parent-timeout') {
        if ($global:PixClipUpdateTest.InstallerCalls -ne 0 -or $global:PixClipUpdateTest.Restarts.Count -ne 0 -or -not $global:PixClipUpdateTest.UpdateError) { throw 'Failed parent timeout' }
    } else {
        $expectedCalls = if ($global:PixClipUpdateTest.CaseName -eq 'tampered') { 0 } else { 1 }
        if ($global:PixClipUpdateTest.InstallerCalls -ne $expectedCalls -or $global:PixClipUpdateTest.Restarts.Count -ne 1 -or $global:PixClipUpdateTest.Restarts[0] -ne $oldExe -or -not $global:PixClipUpdateTest.UpdateError) { throw ("Failed " + $caseName) }
    }
    Write-Output ("Windows updater helper: " + $caseName + " passed (installer and app launch mocked)")
}
