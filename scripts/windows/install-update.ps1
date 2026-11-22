param([Parameter(Mandatory = $true)][string]$Plan)
$ErrorActionPreference = 'Stop'
$relaunch = $null
try {
    $updatePlan = Get-Content -Raw -Encoding UTF8 -LiteralPath $Plan | ConvertFrom-Json
    $parentProcess = Get-Process -Id $updatePlan.pid -ErrorAction SilentlyContinue
    if ($parentProcess -and -not $parentProcess.WaitForExit(120000)) {
        throw 'PixClip did not close. The update was not installed.'
    }
    $relaunch = $updatePlan.original
    $actualHash = (Get-FileHash -LiteralPath $updatePlan.msi -Algorithm SHA256).Hash
    if ($actualHash -ne $updatePlan.hash) { throw 'Installer checksum changed. Update cancelled.' }
    $installer = Join-Path $env:SystemRoot 'System32/msiexec.exe'
    $installerArgs = @('/i', ('"{0}"' -f $updatePlan.msi), '/passive', '/norestart', '/L*v', ('"{0}"' -f $updatePlan.log))
    $result = Start-Process -FilePath $installer -ArgumentList $installerArgs -WindowStyle Hidden -Wait -PassThru
    if ($result.ExitCode -notin @(0, 3010)) { throw "Installation failed ($($result.ExitCode)). Log: $($updatePlan.log)" }
    $installed = (Get-ItemProperty -LiteralPath 'HKCU:\Software\PixClip\Installer' -Name InstallFolder -ErrorAction Stop).InstallFolder
    $relaunch = Join-Path $installed 'PixClip.exe'
} catch {
    if ($updatePlan -and $updatePlan.log) {
        [IO.File]::WriteAllText(($updatePlan.log + '.failed'), $_.Exception.Message)
    }
    $_ | Out-String | Add-Content -LiteralPath ($Plan + '.error.log')
    Add-Type -AssemblyName System.Windows.Forms
    [void][System.Windows.Forms.MessageBox]::Show($_.Exception.Message, 'PixClip update', 'OK', 'Error')
}
if ($relaunch -and (Test-Path -LiteralPath $relaunch)) {
    # The user requested an app restart and needs to see its window.
    Start-Process -FilePath $relaunch
}
