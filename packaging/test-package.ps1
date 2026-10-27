[CmdletBinding()]
param([string]$PortableArchive, [string]$Installer)

# Private fixtures only. Never launch the application or write product registry keys.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'common.ps1')
$repository = Get-BksFullPath (Join-Path $PSScriptRoot '..')
$parent = Join-Path $repository 'target/package-tests'
$null = [IO.Directory]::CreateDirectory($parent)
$sandbox = Join-Path $parent ('.bks-test-' + [Guid]::NewGuid().ToString('N'))
$null = [IO.Directory]::CreateDirectory($sandbox)
$originalAppData = $env:APPDATA
$originalLocalAppData = $env:LOCALAPPDATA
$checks = New-Object 'System.Collections.Generic.List[string]'

function Assert-Check([bool]$Condition, [string]$Name) {
    if (-not $Condition) { throw "FAILED: $Name" }
    $checks.Add($Name)
}
function Assert-Rejected([scriptblock]$Operation, [string]$Name) {
    $rejected = $false
    try { & $Operation | Out-Null } catch { $rejected = $true }
    Assert-Check $rejected $Name
}
function New-FixturePackage([string]$Root, [string]$FixtureVersion) {
    $null = [IO.Directory]::CreateDirectory($Root)
    $mapping = @{
        'install.ps1' = 'scripts/install.ps1'; 'uninstall.ps1' = 'scripts/uninstall.ps1'
        'packaging/common.ps1' = 'packaging/common.ps1'; 'LICENSE' = 'LICENSE'; 'docs/release.md' = 'docs/release.md'
    }
    foreach ($relative in $mapping.Keys) {
        $target = Get-BksChildPath $Root $relative
        $null = [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))
        Copy-Item -LiteralPath (Join-Path $repository $mapping[$relative]) -Destination $target
    }
    [IO.File]::WriteAllText((Join-Path $Root 'bks-app.exe'), 'Fixture only; never executed.')
    [IO.File]::WriteAllText((Join-Path $Root 'dependencies.txt'), 'Fixture dependency report.')
    [IO.File]::WriteAllText((Join-Path $Root 'README.md'), 'Fixture managed documentation.')
    Update-FixtureManifest $Root $FixtureVersion
}
function Update-FixtureManifest([string]$Root, [string]$FixtureVersion) {
    $files = @(Get-ChildItem -LiteralPath $Root -File -Recurse | Where-Object { $_.Name -ne 'package-manifest.json' } |
        Sort-Object FullName | ForEach-Object { [pscustomobject]@{
            path = $_.FullName.Substring($Root.Length + 1).Replace('\', '/')
            bytes = $_.Length; sha256 = Get-BksHash $_.FullName
        } })
    Write-BksJson (Join-Path $Root 'package-manifest.json') @{
        schema = 1; product = 'BKS-Term'; version = $FixtureVersion; architecture = 'x86_64'; files = $files
    }
}
try {
    $env:APPDATA = Join-Path $sandbox 'roaming data'
    $env:LOCALAPPDATA = Join-Path $sandbox 'local data'
    $config = Join-Path $env:APPDATA 'BKS Term'
    $null = [IO.Directory]::CreateDirectory($config)
    [IO.File]::WriteAllText((Join-Path $config 'profiles.json'), '{"fixture":"keep"}')
    [IO.File]::WriteAllText((Join-Path $config 'settings.json'), '{"fixture":"keep"}')
    [IO.File]::WriteAllText((Join-Path $config 'workspace.json'), '{"fixture":"recent profile and folder"}')
    foreach ($name in @('profiles.json.lock', 'settings.json.lock', 'workspace.json.lock')) {
        [IO.File]::WriteAllBytes((Join-Path $config $name), [byte[]]@())
    }
    $profileHash = Get-BksHash (Join-Path $config 'profiles.json')
    $source = Join-Path $sandbox 'payload with spaces'
    $destination = Join-Path $sandbox 'program with spaces/BKS Term'
    New-FixturePackage $source '1.2.3-preview.1'
    & (Join-Path $source 'install.ps1') -InstallDirectory $destination -PayloadDirectory $source -SkipIntegration | Out-Null
    Assert-Check ((Get-BksHash (Join-Path $destination 'bks-app.exe')) -eq (Get-BksHash (Join-Path $source 'bks-app.exe'))) 'install copies verified files into a path with spaces'
    $record = Read-BksInstallation $destination
    Assert-Check ($record.complete -and -not $record.integration) 'fixture install records completeness and skips integration'
    & (Join-Path $source 'install.ps1') -InstallDirectory $destination -PayloadDirectory $source -SkipIntegration | Out-Null
    Assert-Check ((Read-BksInstallation $destination).version -eq '1.2.3-preview.1') 'repeated install is idempotent'
    Assert-Check ((Get-BksHash (Join-Path $config 'profiles.json')) -eq $profileHash) 'install never changes APPDATA profiles'
    $locked = [IO.File]::Open((Join-Path $destination 'bks-app.exe'), [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        Assert-Rejected { & (Join-Path $repository 'scripts/uninstall.ps1') -InstallDirectory $destination -SkipIntegration } 'uninstall rejects locked managed files before removing installation files'
        Assert-Check ((Test-Path -LiteralPath (Join-Path $destination 'uninstall.ps1')) -and
            (Read-BksInstallation $destination).complete) 'failed uninstall leaves its script and ownership record available for retry'
    } finally { $locked.Dispose() }

    # Simulate a crash after the planned marker, before replacing the old executable.
    [IO.File]::AppendAllText((Join-Path $source 'bks-app.exe'), 'Upgraded fixture.')
    Update-FixtureManifest $source '1.2.4-preview.1'
    $interrupted = Read-BksInstallation $destination
    $interrupted.complete = $false
    $interrupted.version = '1.2.4-preview.1'
    $binaryRecord = $interrupted.files | Where-Object { $_.path -eq 'bks-app.exe' } | Select-Object -First 1
    $binaryRecord | Add-Member -NotePropertyName previous_sha256 -NotePropertyValue $binaryRecord.sha256
    $binaryRecord.sha256 = Get-BksHash (Join-Path $source 'bks-app.exe')
    Write-BksJson (Join-Path $destination '.bks-term-install.json') $interrupted
    & (Join-Path $source 'install.ps1') -InstallDirectory $destination -PayloadDirectory $source -SkipIntegration | Out-Null
    Assert-Check ((Read-BksInstallation $destination).complete -and
        (Get-BksHash (Join-Path $destination 'bks-app.exe')) -eq (Get-BksHash (Join-Path $source 'bks-app.exe'))) 'interrupted upgrade accepts its recorded previous hash and completes on retry'

    # Upgrade removes only unchanged obsolete managed files.
    Remove-Item -LiteralPath (Join-Path $source 'README.md')
    [IO.File]::WriteAllText((Join-Path $source 'new-file.txt'), 'New managed upgrade file.')
    Update-FixtureManifest $source '1.2.4-preview.1'
    & (Join-Path $source 'install.ps1') -InstallDirectory $destination -PayloadDirectory $source -SkipIntegration | Out-Null
    Assert-Check ((Read-BksInstallation $destination).version -eq '1.2.4-preview.1' -and
        -not (Test-Path -LiteralPath (Join-Path $destination 'README.md')) -and
        (Test-Path -LiteralPath (Join-Path $destination 'new-file.txt'))) 'upgrade adds current files and removes unchanged obsolete files'

    [IO.File]::WriteAllText((Join-Path $destination 'my-notes.txt'), 'This is not a managed file.')
    [IO.File]::WriteAllText((Join-Path $destination 'docs/release.md'), 'User modified this managed document.')
    Assert-Rejected { & (Join-Path $source 'install.ps1') -InstallDirectory $destination -PayloadDirectory $source -SkipIntegration } 'upgrade refuses to overwrite modified managed files'
    & (Join-Path $repository 'scripts/uninstall.ps1') -InstallDirectory $destination -SkipIntegration | Out-Null
    Assert-Check (-not (Test-Path -LiteralPath (Join-Path $destination 'bks-app.exe')) -and
        (Test-Path -LiteralPath (Join-Path $destination 'my-notes.txt')) -and
        (Test-Path -LiteralPath (Join-Path $destination 'docs/release.md'))) 'uninstall removes own files while preserving unrelated and modified files'
    Assert-Check ((Get-BksHash (Join-Path $config 'profiles.json')) -eq $profileHash -and
        (Test-Path -LiteralPath (Join-Path $config 'settings.json')) -and
        (Test-Path -LiteralPath (Join-Path $config 'workspace.json')) -and
        (Test-Path -LiteralPath (Join-Path $config 'workspace.json.lock'))) 'uninstall preserves profiles, settings, recent workspace and their locks by default'
    & (Join-Path $repository 'scripts/uninstall.ps1') -InstallDirectory $destination -SkipIntegration | Out-Null
    Assert-Check (Test-Path -LiteralPath (Join-Path $destination 'my-notes.txt')) 'repeated uninstall preserves unknown files'
    Assert-Rejected { & (Join-Path $source 'install.ps1') -InstallDirectory $destination -PayloadDirectory $source -SkipIntegration } 'install refuses an unowned nonempty directory'

    $second = Join-Path $sandbox 'second install/BKS Term'
    & (Join-Path $source 'install.ps1') -InstallDirectory $second -PayloadDirectory $source -SkipIntegration | Out-Null
    $partial = Read-BksInstallation $second
    $partial.complete = $false
    $partialBinary = $partial.files | Where-Object { $_.path -eq 'bks-app.exe' } | Select-Object -First 1
    $partialBinary | Add-Member -NotePropertyName previous_sha256 -NotePropertyValue $partialBinary.sha256
    $partialBinary.sha256 = ('0' * 64)
    Write-BksJson (Join-Path $second '.bks-term-install.json') $partial
    [IO.File]::WriteAllText((Join-Path $config 'foreign.txt'), 'Not owned by the app.')
    & (Join-Path $repository 'scripts/uninstall.ps1') -InstallDirectory $second -SkipIntegration -RemoveData | Out-Null
    Assert-Check (-not (Test-Path -LiteralPath (Join-Path $config 'profiles.json')) -and
        -not (Test-Path -LiteralPath (Join-Path $config 'settings.json')) -and
        -not (Test-Path -LiteralPath (Join-Path $config 'workspace.json')) -and
        -not (Test-Path -LiteralPath (Join-Path $config 'profiles.json.lock')) -and
        -not (Test-Path -LiteralPath (Join-Path $config 'settings.json.lock')) -and
        -not (Test-Path -LiteralPath (Join-Path $config 'workspace.json.lock')) -and
        (Test-Path -LiteralPath (Join-Path $config 'foreign.txt'))) 'explicit RemoveData removes known settings while preserving unknown data'
    Assert-Check (-not (Test-Path -LiteralPath $second)) 'uninstall accepts the previous hash of an interrupted upgrade and removes the empty owned directory'
    $third = Join-Path $sandbox 'locked data install/BKS Term'
    & (Join-Path $source 'install.ps1') -InstallDirectory $third -PayloadDirectory $source -SkipIntegration | Out-Null
    $lockPath = Join-Path $config 'profiles.json.lock'
    $dataLock = [IO.File]::Open($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        & (Join-Path $repository 'scripts/uninstall.ps1') -InstallDirectory $third -SkipIntegration -RemoveData | Out-Null
        Assert-Check (Test-Path -LiteralPath $lockPath) 'RemoveData preserves a known lock file while another handle owns it'
    } finally { $dataLock.Dispose() }

    [IO.File]::AppendAllText((Join-Path $source 'bks-app.exe'), 'Tampered.')
    $badDestination = Join-Path $sandbox 'must never be created'
    Assert-Rejected { & (Join-Path $source 'install.ps1') -InstallDirectory $badDestination -PayloadDirectory $source -SkipIntegration } 'tampered payload is rejected before destination writes'
    Assert-Check (-not (Test-Path -LiteralPath $badDestination)) 'hash rejection leaves the destination untouched'
    Update-FixtureManifest $source '1.2.4-preview.1'
    $invalid = Read-BksJson (Join-Path $source 'package-manifest.json')
    $invalid.files[0].path = '../escape.txt'
    Write-BksJson (Join-Path $source 'package-manifest.json') $invalid
    Assert-Rejected { Read-BksPackage $source } 'manifest parent traversal is rejected'
    Assert-Rejected { Get-BksFullPath 'C:\' } 'drive root destination is rejected'
    Assert-Rejected { Get-BksChildPath $sandbox '/outside.txt' } 'rooted manifest path is rejected'
    Assert-Rejected { Get-BksChildPath $sandbox '.bks-term-install.json.' } 'trailing-dot Windows path aliases are rejected'
    Assert-Rejected { Get-BksChildPath $sandbox 'docs/NUL.txt' } 'Windows device names in package paths are rejected'
    $linkTarget = Join-Path $sandbox 'junction target'
    $null = [IO.Directory]::CreateDirectory($linkTarget)
    [IO.File]::WriteAllText((Join-Path $linkTarget 'keep.txt'), 'Outside the linked root; preserve me.')
    $link = Join-Path $sandbox 'fixture junction'
    try {
        $createdLink = $false
        try {
            $null = New-Item -ItemType Junction -Path $link -Value $linkTarget
            $createdLink = $true
        } catch { Write-Output 'SKIPPED: junction fixture creation is unavailable on this machine.' }
        if ($createdLink) {
            Assert-Rejected { Get-BksChildPath $link 'keep.txt' } 'junction-based package and install paths are rejected'
            Assert-Check (Test-Path -LiteralPath (Join-Path $linkTarget 'keep.txt')) 'junction rejection preserves the external target'
        }
    } finally { if ($createdLink) { [IO.Directory]::Delete($link, $false) } }
    $artifactDirectory = Join-Path $sandbox 'artifact output'
    $null = [IO.Directory]::CreateDirectory($artifactDirectory)
    $externalDirectory = Join-Path $sandbox 'external artifact directory'
    $null = [IO.Directory]::CreateDirectory($externalDirectory)
    $externalSentinel = Join-Path $externalDirectory 'external-sentinel.txt'
    [IO.File]::WriteAllText($externalSentinel, 'Never overwrite this external target.')
    $sentinelHash = Get-BksHash $externalSentinel
    Add-Type -TypeDefinition @'
using System.Runtime.InteropServices;
public static class BksPackageTestLinks {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.U1)]
    public static extern bool CreateSymbolicLink(string path, string target, uint flags);
}
'@
    $outputNames = @('test-portable.zip', 'test-setup.exe', 'test-dependencies.txt', 'test-manifest.json', 'test-SHA256SUMS.txt')
    $fileLinksAvailable = $true
    foreach ($name in $outputNames) {
        $outputLink = Join-Path $artifactDirectory $name
        $linked = [BksPackageTestLinks]::CreateSymbolicLink($outputLink, $externalSentinel, 2)
        if (-not $linked) {
            $fileLinksAvailable = $false
            Write-Output ('SKIPPED: file-symlink artifact fixture unavailable, Windows error ' + [Runtime.InteropServices.Marshal]::GetLastWin32Error())
            break
        }
        try {
            Assert-Rejected { Get-BksOutputFile $artifactDirectory $name } ('artifact file symlink is rejected: ' + $name)
            Assert-Check ((Get-BksHash $externalSentinel) -eq $sentinelHash) ('external artifact sentinel is unchanged: ' + $name)
        } finally { [IO.File]::Delete($outputLink) }
    }
    if (-not $fileLinksAvailable) {
        foreach ($name in $outputNames) {
            $outputLink = Join-Path $artifactDirectory $name
            $null = New-Item -ItemType Junction -Path $outputLink -Value $externalDirectory
            try {
                Assert-Rejected { Get-BksOutputFile $artifactDirectory $name } ('artifact junction is rejected: ' + $name)
                Assert-Check ((Get-BksHash $externalSentinel) -eq $sentinelHash) ('external artifact sentinel is unchanged: ' + $name)
            } finally { [IO.Directory]::Delete($outputLink, $false) }
        }
    }
    $ownedExecutable = Join-Path $sandbox 'installed BKS Term/bks-app.exe'
    foreach ($placeholder in @('%1', '%V')) {
        $ownedCommand = '"' + $ownedExecutable + '" --choose-profile --cwd "' + $placeholder + '"'
        Assert-Check (Test-BksExplorerOwnership $ownedExecutable $ownedCommand $ownedExecutable $placeholder) ('Explorer ownership accepts its exact owner and command: ' + $placeholder)
        Assert-Check (-not (Test-BksExplorerOwnership ($ownedExecutable + '.other') $ownedCommand $ownedExecutable $placeholder)) ('Explorer cleanup preserves a foreign owner: ' + $placeholder)
        Assert-Check (-not (Test-BksExplorerOwnership $ownedExecutable ($ownedCommand + ' --profile changed') $ownedExecutable $placeholder)) ('Explorer cleanup preserves a changed command: ' + $placeholder)
    }
    Update-FixtureManifest $source '1.2.4-preview.1'
    $duplicate = Read-BksJson (Join-Path $source 'package-manifest.json')
    $duplicate.files = @($duplicate.files) + @($duplicate.files[0])
    Write-BksJson (Join-Path $source 'package-manifest.json') $duplicate
    Assert-Rejected { Read-BksPackage $source } 'duplicate manifest file paths are rejected'

    Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
    $maliciousArchive = Join-Path $sandbox 'zip-slip.zip'
    $zip = [IO.Compression.ZipFile]::Open($maliciousArchive, [IO.Compression.ZipArchiveMode]::Create)
    try { $null = $zip.CreateEntry('../escaped.txt') } finally { $zip.Dispose() }
    $zipDestination = Join-Path $sandbox 'zip-slip-extract'
    $null = [IO.Directory]::CreateDirectory($zipDestination)
    Assert-Rejected { Expand-BksPayload $maliciousArchive $zipDestination } 'archive parent traversal is rejected'
    Assert-Check (-not (Test-Path -LiteralPath (Join-Path $sandbox 'escaped.txt'))) 'archive cannot write outside extraction root'

    if ($PortableArchive) {
        $real = Join-Path $sandbox 'actual portable'
        $null = [IO.Directory]::CreateDirectory($real)
        Expand-BksPayload ([IO.Path]::GetFullPath($PortableArchive)) $real
        $null = Read-BksPackage $real
        $realInstall = Join-Path $sandbox 'actual install/BKS Term'
        & (Join-Path $real 'install.ps1') -InstallDirectory $realInstall -PayloadDirectory $real -SkipIntegration | Out-Null
        Assert-Check ((Get-BksHash (Join-Path $realInstall 'bks-app.exe')) -eq (Get-BksHash (Join-Path $real 'bks-app.exe'))) 'actual portable archive installs its exact executable without launching it'
        & (Join-Path $realInstall 'uninstall.ps1') -InstallDirectory $realInstall -SkipIntegration | Out-Null
        Assert-Check (-not (Test-Path -LiteralPath $realInstall)) 'actual package uninstalls without touching real user integration'
        if (-not $Installer -and $PortableArchive.EndsWith('-portable.zip', [StringComparison]::OrdinalIgnoreCase)) {
            $candidate = $PortableArchive.Substring(0, $PortableArchive.Length - '-portable.zip'.Length) + '-setup.exe'
            if (Test-Path -LiteralPath $candidate -PathType Leaf) { $Installer = [IO.Path]::GetFullPath($candidate) }
        }
        if ($Installer) {
            $sfxInstall = Join-Path $sandbox 'setup install with spaces/BKS Term'
            $stderr = Join-Path $sandbox 'setup-stderr.txt'
            $process = Start-Process -FilePath ([IO.Path]::GetFullPath($Installer)) -ArgumentList @(
                '--quiet', '--skip-integration', '--install-directory', ('"' + $sfxInstall + '"')
            ) -WindowStyle Hidden -RedirectStandardError $stderr -PassThru -Wait
            if ($process.ExitCode -ne 0) { throw ('Actual setup failed in isolated fixture: ' + [IO.File]::ReadAllText($stderr)) }
            Assert-Check ((Get-BksHash (Join-Path $sfxInstall 'bks-app.exe')) -eq (Get-BksHash (Join-Path $real 'bks-app.exe'))) 'actual setup EXE installs into a private directory without integration or launching the application'
            & (Join-Path $sfxInstall 'uninstall.ps1') -InstallDirectory $sfxInstall -SkipIntegration | Out-Null
            Assert-Check (-not (Test-Path -LiteralPath $sfxInstall)) 'actual setup installation uninstalls cleanly'
        } else { Write-Output 'SKIPPED: setup EXE smoke (supply -Installer or a sibling -setup.exe).' }
    }
    Write-Output ("PASSED: {0} package/install/uninstall checks; product registry/shortcuts were skipped." -f $checks.Count)
    $checks | ForEach-Object { Write-Output "  passed: $_" }
} finally {
    $env:APPDATA = $originalAppData
    $env:LOCALAPPDATA = $originalLocalAppData
    Remove-BksPrivateTree $sandbox $parent '.bks-test-'
}
