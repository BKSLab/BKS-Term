[CmdletBinding()]
param(
    [string]$InstallDirectory = (Join-Path $env:LOCALAPPDATA 'Programs/BKS Term'),
    [string]$PayloadDirectory = $PSScriptRoot,
    [string]$PayloadArchive,
    [switch]$SkipIntegration
)

$ErrorActionPreference = 'Stop'
$common = @('packaging/common.ps1', 'common.ps1', '../packaging/common.ps1') |
    ForEach-Object { Join-Path $PSScriptRoot $_ } | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
if (-not $common) { throw 'The BKS Term packaging helper is missing.' }
. $common
$temporary = $null
try {
    $destination = Get-BksFullPath $InstallDirectory
    Assert-BksNoReparse $destination
    if ($PayloadArchive) {
        $temporary = Join-Path ([IO.Path]::GetTempPath()) ('.bks-install-' + [Guid]::NewGuid().ToString('N'))
        $null = [IO.Directory]::CreateDirectory($temporary)
        Expand-BksPayload ([IO.Path]::GetFullPath($PayloadArchive)) $temporary
        $PayloadDirectory = $temporary
    }
    $source = Get-BksFullPath $PayloadDirectory
    Assert-BksNoReparse $source
    if ([string]::Equals($source, $destination, [StringComparison]::OrdinalIgnoreCase) -or
        $source.StartsWith($destination + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Run the installer from outside the installation directory.'
    }
    $package = Read-BksPackage $source
    $previous = Read-BksInstallation $destination
    if (-not $previous -and (Test-Path -LiteralPath $destination) -and @(Get-ChildItem -LiteralPath $destination -Force).Count) {
        throw 'The destination is not empty and is not an existing BKS Term installation.'
    }
    $files = @($package.files) + @([pscustomobject]@{
        path = 'package-manifest.json'; sha256 = Get-BksHash (Join-Path $source 'package-manifest.json')
    })
    $oldByPath = @{}
    if ($previous) { foreach ($file in $previous.files) { $oldByPath[$file.path] = $file } }
    foreach ($file in $files) {
        $target = Get-BksChildPath $destination $file.path
        if (Test-Path -LiteralPath $target) {
            if (-not (Test-Path -LiteralPath $target -PathType Leaf)) { throw "A directory blocks the package file: $($file.path)" }
            $existingHash = Get-BksHash $target
            $wasManaged = $oldByPath.ContainsKey($file.path) -and (Test-BksInstalledHash $oldByPath[$file.path] $existingHash)
            if ($existingHash -ne $file.sha256 -and -not $wasManaged) {
                throw "A modified or unrelated file would be overwritten: $($file.path)"
            }
        }
    }
    $shortcutPath = $null
    $shell = $null
    $uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\BKS Term'
    if (-not $SkipIntegration) {
        $shortcutPath = Get-BksShortcutPath
        $shell = New-Object -ComObject WScript.Shell
        if (Test-Path -LiteralPath $shortcutPath) {
            if (-not [string]::Equals($shell.CreateShortcut($shortcutPath).TargetPath,
                    (Join-Path $destination 'bks-app.exe'), [StringComparison]::OrdinalIgnoreCase)) {
                throw 'An unrelated Start Menu shortcut already uses the name BKS Term.'
            }
        }
        if (Test-Path -LiteralPath $uninstallKey) {
            $registered = (Get-ItemProperty -LiteralPath $uninstallKey).InstallLocation
            if (-not [string]::Equals($registered, $destination, [StringComparison]::OrdinalIgnoreCase)) {
                throw 'Another BKS Term directory is registered. Uninstall it before choosing a different directory.'
            }
        }
    }
    $null = [IO.Directory]::CreateDirectory($destination)
    # An interrupted copy remains recognizable and can be retried or uninstalled.
    $planned = @{}
    foreach ($file in $oldByPath.Values) { $planned[$file.path] = $file }
    foreach ($file in $files) {
        $plannedFile = [ordered]@{ path = $file.path; sha256 = $file.sha256 }
        $target = Get-BksChildPath $destination $file.path
        if (Test-Path -LiteralPath $target -PathType Leaf) {
            $existingHash = Get-BksHash $target
            if ($existingHash -ne $file.sha256) { $plannedFile.previous_sha256 = $existingHash }
        }
        $planned[$file.path] = [pscustomobject]$plannedFile
    }
    $record = [ordered]@{
        schema = 1; product = 'BKS-Term'; version = $package.version; install_root = $destination
        complete = $false; integration = ((-not [bool]$SkipIntegration) -or ($previous -and $previous.integration)); files = @($planned.Values)
    }
    $recordPath = Get-BksChildPath $destination '.bks-term-install.json'
    Write-BksJson $recordPath $record
    foreach ($file in $files) {
        $target = Get-BksChildPath $destination $file.path
        $null = [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))
        Copy-Item -LiteralPath (Get-BksChildPath $source $file.path) -Destination $target -Force
    }
    foreach ($old in $oldByPath.Values) {
        if (@($files | Where-Object { $_.path -eq $old.path }).Count) { continue }
        $target = Get-BksChildPath $destination $old.path
        if ((Test-Path -LiteralPath $target -PathType Leaf) -and (Get-BksHash $target) -eq $old.sha256) {
            Remove-Item -LiteralPath $target -Force
            Remove-BksEmptyParents $target $destination
        }
    }
    if (-not $SkipIntegration) {
        $null = [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($shortcutPath))
        $shortcut = $shell.CreateShortcut($shortcutPath)
        $shortcut.TargetPath = Join-Path $destination 'bks-app.exe'
        $shortcut.WorkingDirectory = $destination
        $shortcut.Description = 'BKS Term accessible Windows terminal'
        $shortcut.Save()
        $null = New-Item -Path $uninstallKey -Force
        $uninstallCommand = '"' + (Get-BksPowerShell) + '" -NoProfile -ExecutionPolicy Bypass -File "' +
            (Join-Path $destination 'uninstall.ps1') + '" -InstallDirectory "' + $destination + '"'
        $properties = @{
            DisplayName = 'BKS Term'; DisplayVersion = $package.version; Publisher = 'BKS Term contributors'
            InstallLocation = $destination; DisplayIcon = (Join-Path $destination 'bks-app.exe')
            UninstallString = $uninstallCommand; QuietUninstallString = $uninstallCommand
        }
        foreach ($name in $properties.Keys) { $null = New-ItemProperty -LiteralPath $uninstallKey -Name $name -Value $properties[$name] -PropertyType String -Force }
        foreach ($name in @('NoModify', 'NoRepair')) { $null = New-ItemProperty -LiteralPath $uninstallKey -Name $name -Value 1 -PropertyType DWord -Force }
    }
    $record.complete = $true
    $record.files = $files
    Write-BksJson $recordPath $record
    Write-Output "BKS Term $($package.version) installed for the current user: $destination"
    Write-Output 'Profiles and settings in APPDATA were preserved. Close the application before upgrading.'
} finally {
    if ($temporary) { Remove-BksPrivateTree $temporary ([IO.Path]::GetTempPath()) '.bks-install-' }
}
