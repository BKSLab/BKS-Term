# Shared by the portable installer and uninstaller. Windows PowerShell 5.1+.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-BksFullPath([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path)) { throw 'An explicit, nonempty path is required.' }
    $full = [IO.Path]::GetFullPath($Path).TrimEnd('\')
    if ($full -notmatch '^[A-Za-z]:\\.+') { throw "A local directory below a drive root is required: $Path" }
    return $full
}

function Assert-BksNoReparse([string]$Path) {
    $current = [IO.Path]::GetFullPath($Path)
    while ($current) {
        $item = Get-Item -LiteralPath $current -Force -ErrorAction SilentlyContinue
        if ($item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Refusing a junction or symbolic link: $current"
        }
        $parent = [IO.Path]::GetDirectoryName($current.TrimEnd('\'))
        if ($parent -eq $current) { break }
        $current = $parent
    }
}

function Get-BksChildPath([string]$Root, [string]$Relative) {
    $rootPath = Get-BksFullPath $Root
    if ([string]::IsNullOrWhiteSpace($Relative) -or $Relative -match '[\x00-\x1f\\:*?"<>|]' -or
        [IO.Path]::IsPathRooted($Relative) -or @($Relative.Split('/') | Where-Object { $_ -in @('', '.', '..') }).Count) {
        throw "Unsafe manifest path: $Relative"
    }
    foreach ($segment in $Relative.Split('/')) {
        if ($segment.TrimEnd(' ', '.') -cne $segment -or $segment -match '^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\..*)?$') {
            throw "Reserved or ambiguous Windows manifest path: $Relative"
        }
    }
    $child = [IO.Path]::GetFullPath((Join-Path $rootPath $Relative))
    if (-not $child.StartsWith($rootPath + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Path leaves its package directory: $Relative"
    }
    Assert-BksNoReparse $child
    return $child
}

function Get-BksHash([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-BksOutputFile([string]$Root, [string]$Relative) {
    $path = Get-BksChildPath $Root $Relative
    if ((Test-Path -LiteralPath $path) -and -not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "An output file is blocked by a directory: $path"
    }
    return $path
}

function Test-BksExplorerOwnership([string]$Owner, [string]$Command, [string]$Executable, [string]$Placeholder) {
    if ($Placeholder -notin @('%1', '%V')) { return $false }
    $expected = '"' + $Executable + '" --choose-profile --cwd "' + $Placeholder + '"'
    return [string]::Equals($Owner, $Executable, [StringComparison]::Ordinal) -and
        [string]::Equals($Command, $expected, [StringComparison]::Ordinal)
}

function Remove-BksExplorerIntegration([string]$Executable) {
    # Fixed HKCU paths mirror desktop.rs. Called only when SkipIntegration is false.
    $verbs = @(
        @{ key = 'Software\Classes\Directory\shell\BKSTerm'; placeholder = '%1' },
        @{ key = 'Software\Classes\Directory\Background\shell\BKSTerm'; placeholder = '%V' }
    )
    $removed = $false
    foreach ($verb in $verbs) {
        $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($verb.key, $false)
        if (-not $key) { continue }
        $owned = $false
        $commandKey = $null
        try {
            $commandKey = $key.OpenSubKey('command', $false)
            if ($commandKey -and $key.GetValueNames() -contains 'BksTermOwnerExecutable' -and
                $commandKey.GetValueNames() -contains '' -and
                $key.GetValueKind('BksTermOwnerExecutable') -eq [Microsoft.Win32.RegistryValueKind]::String -and
                $commandKey.GetValueKind('') -eq [Microsoft.Win32.RegistryValueKind]::String) {
                $owned = Test-BksExplorerOwnership ($key.GetValue('BksTermOwnerExecutable', '')) ($commandKey.GetValue('', '')) $Executable $verb.placeholder
            }
        } finally {
            if ($commandKey) { $commandKey.Dispose() }
            $key.Dispose()
        }
        if ($owned) {
            [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($verb.key, $false)
            $removed = $true
        } else { Write-Output ('Unrelated or modified Explorer verb preserved: HKCU\' + $verb.key) }
    }
    if ($removed) {
        if (-not ('BksTermUninstallShell' -as [type])) {
            Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class BksTermUninstallShell {
    [DllImport("shell32.dll")] public static extern void SHChangeNotify(uint eventId, uint flags, IntPtr item1, IntPtr item2);
}
'@
        }
        [BksTermUninstallShell]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)
    }
}

function Read-BksJson([string]$Path) {
    Assert-BksNoReparse $Path
    return [IO.File]::ReadAllText($Path, [Text.Encoding]::UTF8) | ConvertFrom-Json
}

function Write-BksJson([string]$Path, $Value) {
    Assert-BksNoReparse $Path
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 10) + "`r`n", (New-Object Text.UTF8Encoding $false))
}

function Read-BksPackage([string]$Root) {
    $manifestPath = Get-BksChildPath $Root 'package-manifest.json'
    $manifest = Read-BksJson $manifestPath
    if ($manifest.schema -ne 1 -or $manifest.product -ne 'BKS-Term' -or
        $manifest.version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$' -or $manifest.architecture -ne 'x86_64') {
        throw 'The package manifest has an unsupported format, product, version or architecture.'
    }
    $seen = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    foreach ($file in $manifest.files) {
        $path = Get-BksChildPath $Root $file.path
        if (-not $seen.Add($file.path) -or $file.path -in @('package-manifest.json', '.bks-term-install.json') -or
            $file.sha256 -notmatch '^[0-9a-f]{64}$') { throw "Invalid or duplicate file record: $($file.path)" }
        if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or
            (Get-Item -LiteralPath $path).Length -ne $file.bytes -or (Get-BksHash $path) -ne $file.sha256) {
            throw "Package SHA256/size verification failed: $($file.path)"
        }
    }
    foreach ($required in @('bks-app.exe', 'install.ps1', 'uninstall.ps1', 'packaging/common.ps1', 'LICENSE', 'docs/release.md', 'dependencies.txt')) {
        if (-not $seen.Contains($required)) { throw "Required package file is absent: $required" }
    }
    return $manifest
}

function Read-BksInstallation([string]$Root) {
    $path = Get-BksChildPath $Root '.bks-term-install.json'
    if (-not (Test-Path -LiteralPath $path)) { return $null }
    $record = Read-BksJson $path
    if ($record.schema -ne 1 -or $record.product -ne 'BKS-Term' -or
        -not [string]::Equals($record.install_root, (Get-BksFullPath $Root), [StringComparison]::OrdinalIgnoreCase)) {
        throw 'This directory does not have a matching BKS Term installation record.'
    }
    $seen = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    foreach ($file in $record.files) {
        $null = Get-BksChildPath $Root $file.path
        if (-not $seen.Add($file.path) -or $file.path -eq '.bks-term-install.json' -or
            $file.sha256 -notmatch '^[0-9a-f]{64}$') { throw 'Invalid installation file record.' }
        if ($file.PSObject.Properties['previous_sha256'] -and $file.previous_sha256 -notmatch '^[0-9a-f]{64}$') {
            throw 'Invalid interrupted-update hash record.'
        }
    }
    return $record
}

function Test-BksInstalledHash($File, [string]$Hash) {
    return $File.sha256 -eq $Hash -or ($File.PSObject.Properties['previous_sha256'] -and $File.previous_sha256 -eq $Hash)
}

function Expand-BksPayload([string]$Archive, [string]$Destination) {
    Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        $seen = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
        foreach ($entry in $zip.Entries) {
            $relative = $entry.FullName.TrimEnd('/')
            if (-not $relative) { continue }
            $target = Get-BksChildPath $Destination $relative
            if (-not $seen.Add($relative)) { throw "Duplicate archive entry: $relative" }
            if ($entry.FullName.EndsWith('/')) {
                $null = [IO.Directory]::CreateDirectory($target)
            } else {
                $null = [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))
                [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $target, $false)
            }
        }
    } finally { $zip.Dispose() }
}

# Only private directories just created by these scripts may be passed here.
# Each removal is literal and nonrecursive; junctions are rejected before enumeration.
function Remove-BksPrivateTree([string]$Root, [string]$ExpectedParent, [string]$Prefix) {
    $full = Get-BksFullPath $Root
    $parent = Get-BksFullPath $ExpectedParent
    if (-not [string]::Equals([IO.Path]::GetDirectoryName($full), $parent, [StringComparison]::OrdinalIgnoreCase) -or
        -not [IO.Path]::GetFileName($full).StartsWith($Prefix, [StringComparison]::Ordinal)) {
        throw 'Refusing cleanup outside the private temporary directory.'
    }
    if (-not (Test-Path -LiteralPath $full)) { return }
    Assert-BksNoReparse $full
    $pending = New-Object 'System.Collections.Generic.Stack[string]'
    $directories = New-Object 'System.Collections.Generic.List[string]'
    $files = New-Object 'System.Collections.Generic.List[string]'
    $pending.Push($full)
    while ($pending.Count) {
        $directory = $pending.Pop()
        $directories.Add($directory)
        foreach ($item in Get-ChildItem -LiteralPath $directory -Force) {
            Assert-BksNoReparse $item.FullName
            if ($item.PSIsContainer) { $pending.Push($item.FullName) } else { $files.Add($item.FullName) }
        }
    }
    foreach ($file in $files) { Remove-Item -LiteralPath $file -Force }
    for ($index = $directories.Count - 1; $index -ge 0; $index--) { [IO.Directory]::Delete($directories[$index], $false) }
}

function Remove-BksEmptyParents([string]$Path, [string]$Root) {
    $rootPath = Get-BksFullPath $Root
    $parent = [IO.Path]::GetDirectoryName($Path)
    while ($parent -and $parent.StartsWith($rootPath + '\', [StringComparison]::OrdinalIgnoreCase)) {
        Assert-BksNoReparse $parent
        if (Test-Path -LiteralPath $parent) {
            if (@(Get-ChildItem -LiteralPath $parent -Force).Count) { break }
            [IO.Directory]::Delete($parent, $false)
        }
        $parent = [IO.Path]::GetDirectoryName($parent)
    }
}

function Get-BksShortcutPath {
    $programs = [Environment]::GetFolderPath('Programs')
    if (-not $programs) { throw 'The current user Start Menu directory is unavailable.' }
    $path = Join-Path $programs 'BKS Term.lnk'
    Assert-BksNoReparse $path
    return $path
}

function Get-BksPowerShell {
    return (Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe')
}
