[CmdletBinding()]
param(
    [string]$InstallDirectory = (Join-Path $env:LOCALAPPDATA 'Programs/BKS Term'),
    [switch]$RemoveData,
    [switch]$SkipIntegration
)

$ErrorActionPreference = 'Stop'
$common = @('packaging/common.ps1', 'common.ps1', '../packaging/common.ps1') |
    ForEach-Object { Join-Path $PSScriptRoot $_ } | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
if (-not $common) { throw 'The BKS Term packaging helper is missing.' }
. $common
$destination = Get-BksFullPath $InstallDirectory
Assert-BksNoReparse $destination
$record = Read-BksInstallation $destination
if (-not $record) {
    Write-Output 'No matching BKS Term installation record; no files or data were removed.'
    return
}
$retained = New-Object 'System.Collections.Generic.List[string]'
$removals = New-Object 'System.Collections.Generic.List[string]'
foreach ($file in $record.files) {
    $target = Get-BksChildPath $destination $file.path
    if (-not (Test-Path -LiteralPath $target)) { continue }
    if ((Test-Path -LiteralPath $target -PathType Leaf) -and (Test-BksInstalledHash $file (Get-BksHash $target))) {
        # Check locked/executing files before changing the installation or its registration.
        try {
            $probe = [IO.File]::Open($target, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
            $probe.Dispose()
        } catch { throw "Close BKS Term and applications using its files before uninstalling: $target" }
        $removals.Add($target)
    } else { $retained.Add($file.path) }
}
# Delete the executable first and the running PowerShell script/helper last.
$orderedRemovals = $removals | Sort-Object @{ Expression = {
    $relative = $_.Substring($destination.Length + 1).Replace('\', '/')
    if ($relative -eq 'bks-app.exe') { 0 } elseif ($relative -eq 'packaging/common.ps1') { 2 }
    elseif ($relative -eq 'uninstall.ps1') { 3 } else { 1 }
} }
foreach ($target in $orderedRemovals) {
    Remove-Item -LiteralPath $target -Force
    Remove-BksEmptyParents $target $destination
}
if (-not $SkipIntegration) {
    # Explorer opt-in can happen after an installation that skipped integration.
    Remove-BksExplorerIntegration (Join-Path $destination 'bks-app.exe')
}
if (-not $SkipIntegration -and $record.integration) {
    $shortcutPath = Get-BksShortcutPath
    if (Test-Path -LiteralPath $shortcutPath) {
        $shell = New-Object -ComObject WScript.Shell
        if ([string]::Equals($shell.CreateShortcut($shortcutPath).TargetPath,
                (Join-Path $destination 'bks-app.exe'), [StringComparison]::OrdinalIgnoreCase)) {
            Remove-Item -LiteralPath $shortcutPath -Force
        }
    }
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\BKS Term'
    if (Test-Path -LiteralPath $key) {
        if ([string]::Equals((Get-ItemProperty -LiteralPath $key).InstallLocation,
                $destination, [StringComparison]::OrdinalIgnoreCase)) { Remove-Item -LiteralPath $key }
    }
}
Remove-Item -LiteralPath (Get-BksChildPath $destination '.bks-term-install.json') -Force
if ($RemoveData) {
    $configuration = Get-BksFullPath (Join-Path $env:APPDATA 'BKS Term')
    Assert-BksNoReparse $configuration
    # Deliberately retain unknown files even when RemoveData was requested.
    foreach ($name in @('profiles.json', 'settings.json', 'workspace.json')) {
        $path = Get-BksChildPath $configuration $name
        if (Test-Path -LiteralPath $path -PathType Leaf) { Remove-Item -LiteralPath $path -Force }
    }
    foreach ($name in @('profiles.json.lock', 'settings.json.lock', 'workspace.json.lock')) {
        $path = Get-BksChildPath $configuration $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { continue }
        try {
            $probe = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
            $probe.Dispose()
            Remove-Item -LiteralPath $path -Force
        } catch { Write-Output "In-use configuration lock preserved: $name" }
    }
    if ((Test-Path -LiteralPath $configuration) -and -not @(Get-ChildItem -LiteralPath $configuration -Force).Count) {
        [IO.Directory]::Delete($configuration, $false)
    }
}
if (-not @(Get-ChildItem -LiteralPath $destination -Force).Count) { [IO.Directory]::Delete($destination, $false) }
Write-Output 'BKS Term removed. Unrelated or modified files were preserved.'
if (-not $RemoveData) { Write-Output 'Profiles and settings in APPDATA were preserved (use -RemoveData for their explicit deletion).' }
if ($retained.Count) { Write-Output ('Modified package files retained: ' + ($retained -join ', ')) }
