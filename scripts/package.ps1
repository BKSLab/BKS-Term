[CmdletBinding()]
param(
    [string]$Exe = (Join-Path $PSScriptRoot '../target/release/bks-app.exe'),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../target/packages'),
    [string]$Version,
    [string]$Dumpbin,
    [string]$Csc = (Join-Path $env:SystemRoot 'Microsoft.NET/Framework64/v4.0.30319/csc.exe')
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../packaging/common.ps1')
$repository = Get-BksFullPath (Join-Path $PSScriptRoot '..')
$binary = Get-BksFullPath $Exe
$output = Get-BksFullPath $OutputDirectory
Assert-BksNoReparse $binary
Assert-BksNoReparse $output
if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw "Build the release executable first: $binary" }
$cargo = [IO.File]::ReadAllText((Join-Path $repository 'crates/bks-app/Cargo.toml'))
$cargoVersion = [regex]::Match($cargo, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $Version) { $Version = $cargoVersion }
if ($Version -cne $cargoVersion -or $Version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') {
    throw 'Version must match the bks-app Cargo package version (including any prerelease suffix).'
}
$productVersion = (Get-Item -LiteralPath $binary).VersionInfo.ProductVersion
if ($productVersion -cne $Version) { throw "The executable ProductVersion '$productVersion' does not match Cargo '$Version'. Rebuild release before packaging." }
$bytes = [IO.File]::ReadAllBytes($binary)
if ($bytes.Length -lt 256 -or [BitConverter]::ToUInt16($bytes, 0) -ne 0x5a4d) { throw 'The input is not a PE executable.' }
$peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
if ($peOffset -lt 0 -or $peOffset + 6 -gt $bytes.Length -or [BitConverter]::ToUInt32($bytes, $peOffset) -ne 0x4550 -or
    [BitConverter]::ToUInt16($bytes, $peOffset + 4) -ne 0x8664) { throw 'Only x86_64 Windows PE binaries are packaged.' }
if (-not $Dumpbin) {
    $command = Get-Command dumpbin.exe -ErrorAction SilentlyContinue
    if ($command) { $Dumpbin = $command.Source }
}
if (-not $Dumpbin) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    if (Test-Path -LiteralPath $vswhere) {
        $visualStudio = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($visualStudio) {
            $Dumpbin = Get-ChildItem -LiteralPath (Join-Path $visualStudio 'VC/Tools/MSVC') -Directory |
                Sort-Object Name -Descending | ForEach-Object { Join-Path $_.FullName 'bin/Hostx64/x64/dumpbin.exe' } |
                Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
        }
    }
}
if (-not $Dumpbin -or -not (Test-Path -LiteralPath $Dumpbin -PathType Leaf)) { throw 'dumpbin.exe is required for the release dependency report.' }
if (-not (Test-Path -LiteralPath $Csc -PathType Leaf)) { throw 'The Windows .NET Framework C# compiler is required to build the setup executable.' }
$dependencyOutput = @(& $Dumpbin /nologo /dependents $binary 2>&1)
if ($LASTEXITCODE -ne 0) { throw 'dumpbin /dependents failed.' }
$dlls = @($dependencyOutput | ForEach-Object {
    if ("$_" -match '^\s+([A-Za-z0-9_.-]+\.dll)\s*$') { $matches[1] }
} | Sort-Object -Unique)
if (-not $dlls.Count) { throw 'dumpbin did not report any imported DLLs.' }
$externalRuntime = @($dlls | Where-Object { $_ -match '^(?:VCRUNTIME\d|MSVCP\d|MSVCR\d|MSVCRTD|CONCRT\d|VCOMP\d|ucrtbased)' })
if ($externalRuntime.Count) {
    throw ('Release imports the Visual C++ redistributable/debug runtime. Build with -C target-feature=+crt-static: ' + ($externalRuntime -join ', '))
}
$null = [IO.Directory]::CreateDirectory($output)
# Keep a space in the private build path so release builds exercise quoting too.
$work = Join-Path $output ('.bks-package-' + [Guid]::NewGuid().ToString('N') + ' with spaces')
$null = [IO.Directory]::CreateDirectory($work)
try {
    $payload = Join-Path $work 'payload'
    $null = [IO.Directory]::CreateDirectory($payload)
    $sources = [ordered]@{
        'bks-app.exe' = $binary
        'install.ps1' = (Join-Path $repository 'scripts/install.ps1')
        'uninstall.ps1' = (Join-Path $repository 'scripts/uninstall.ps1')
        'packaging/common.ps1' = (Join-Path $repository 'packaging/common.ps1')
        'LICENSE' = (Join-Path $repository 'LICENSE')
        'README.md' = (Join-Path $repository 'README.md')
        'CHANGELOG.md' = (Join-Path $repository 'CHANGELOG.md')
        'docs/help.md' = (Join-Path $repository 'docs/help.md')
        'docs/known-limitations.md' = (Join-Path $repository 'docs/known-limitations.md')
        'docs/release.md' = (Join-Path $repository 'docs/release.md')
    }
    foreach ($manual in Get-ChildItem -LiteralPath (Join-Path $repository 'docs') -Filter 'manual-*.md' -File) {
        $sources['docs/' + $manual.Name] = $manual.FullName
    }
    foreach ($relative in $sources.Keys) {
        $target = Get-BksChildPath $payload $relative
        if (-not (Test-Path -LiteralPath $sources[$relative] -PathType Leaf)) { throw "Required release source is missing: $relative" }
        Assert-BksNoReparse $sources[$relative]
        $null = [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))
        Copy-Item -LiteralPath $sources[$relative] -Destination $target
    }
    $binaryHash = Get-BksHash (Join-Path $payload 'bks-app.exe')
    $dependencies = "BKS Term $Version, x86_64`r`nExecutable SHA256: $binaryHash`r`n" +
        "Direct PE imports (dumpbin /dependents); no DLL files are redistributed by this package.`r`n" +
        "Imports must be supplied by the supported Windows installation.`r`n" +
        "VC redistributable/debug DLL imports: none. This is not a clean-VM execution test.`r`n`r`n" + ($dependencyOutput -join "`r`n") + "`r`n"
    [IO.File]::WriteAllText((Join-Path $payload 'dependencies.txt'), $dependencies, (New-Object Text.UTF8Encoding $false))
    $fileRecords = @(Get-ChildItem -LiteralPath $payload -File -Recurse | Sort-Object FullName | ForEach-Object {
        [pscustomobject]@{
            path = $_.FullName.Substring($payload.Length + 1).Replace('\', '/')
            sha256 = Get-BksHash $_.FullName
            bytes = $_.Length
        }
    })
    $signature = Get-AuthenticodeSignature -LiteralPath (Join-Path $payload 'bks-app.exe')
    $signatureStatus = if ($signature.Status -eq 'NotSigned') { 'unsigned' } else { "$($signature.Status)" }
    $manifest = [ordered]@{
        schema = 1; product = 'BKS-Term'; version = $Version; architecture = 'x86_64'
        platform = 'Windows 10/11 with ConPTY'; executable = 'bks-app.exe'
        packaged_at_utc = [DateTime]::UtcNow.ToString('o'); executable_signature = $signatureStatus
        files = $fileRecords
    }
    Write-BksJson (Join-Path $payload 'package-manifest.json') $manifest
    $null = Read-BksPackage $payload
    $stem = 'bks-term-' + $Version + '-windows-x64'
    $archive = Join-Path $work 'payload.zip'
    Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
    # Framework CreateFromDirectory emits backslashes on Windows. Write the ZIP
    # entry names explicitly so the package uses the portable '/' convention.
    $zip = [IO.Compression.ZipFile]::Open($archive, [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($file in $fileRecords) {
            $null = [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip,
                (Get-BksChildPath $payload $file.path), $file.path, [IO.Compression.CompressionLevel]::Optimal)
        }
        $null = [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip,
            (Join-Path $payload 'package-manifest.json'), 'package-manifest.json', [IO.Compression.CompressionLevel]::Optimal)
    } finally { $zip.Dispose() }
    # The setup embeds the ZIP and bootstrap scripts, with an explicit asInvoker manifest.
    Copy-Item -LiteralPath (Join-Path $repository 'scripts/install.ps1') -Destination (Join-Path $work 'install.ps1')
    Copy-Item -LiteralPath (Join-Path $repository 'packaging/common.ps1') -Destination (Join-Path $work 'common.ps1')
    $setup = Join-Path $work ($stem + '-setup.exe')
    $numericVersion = ($Version -split '-')[0] + '.0'
    $assemblyInfo = 'using System.Reflection;' + "`r`n" +
        '[assembly: AssemblyTitle("BKS Term setup")]' + "`r`n" +
        '[assembly: AssemblyProduct("BKS Term")]' + "`r`n" +
        '[assembly: AssemblyCompany("BKS Term contributors")]' + "`r`n" +
        '[assembly: AssemblyFileVersion("' + $numericVersion + '")]' + "`r`n" +
        '[assembly: AssemblyInformationalVersion("' + $Version + '")]' + "`r`n"
    $assemblyPath = Join-Path $work 'AssemblyInfo.cs'
    [IO.File]::WriteAllText($assemblyPath, $assemblyInfo, (New-Object Text.UTF8Encoding $false))
    $cscArguments = @(
        '/nologo', '/target:winexe', '/platform:x64', '/optimize+', '/warnaserror+', '/reference:System.Windows.Forms.dll',
        ('/win32manifest:' + (Join-Path $repository 'packaging/installer.manifest')),
        ('/resource:' + $archive + ',BksTerm.Payload'),
        ('/resource:' + (Join-Path $work 'install.ps1') + ',BksTerm.Install'),
        ('/resource:' + (Join-Path $work 'common.ps1') + ',BksTerm.Common'),
        ('/out:' + $setup), (Join-Path $repository 'packaging/Installer.cs'), $assemblyPath
    )
    & $Csc @cscArguments
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $setup -PathType Leaf)) {
        throw 'The C# compiler failed to create the setup executable.'
    }
    # Extraction-only smoke test deliberately prevents the install command.
    $extract = Join-Path $work 'sfx-extraction'
    $null = [IO.Directory]::CreateDirectory($extract)
    $smoke = Start-Process -FilePath $setup -ArgumentList @('--extract-only', ('"' + $extract + '"')) -WindowStyle Hidden -PassThru -Wait
    if ($smoke.ExitCode -ne 0 -or -not (Test-Path -LiteralPath (Join-Path $extract 'payload.zip')) -or
        (Get-BksHash (Join-Path $extract 'payload.zip')) -ne (Get-BksHash $archive) -or
        (Get-BksHash (Join-Path $extract 'install.ps1')) -ne (Get-BksHash (Join-Path $work 'install.ps1')) -or
        (Get-BksHash (Join-Path $extract 'common.ps1')) -ne (Get-BksHash (Join-Path $work 'common.ps1'))) {
        throw 'The installer extraction-only smoke test failed.'
    }
    # Validate every final destination before publishing any artifact, and again at its write.
    $portableOutput = Get-BksOutputFile $output ($stem + '-portable.zip')
    $setupOutput = Get-BksOutputFile $output ($stem + '-setup.exe')
    $dependenciesOutput = Get-BksOutputFile $output ($stem + '-dependencies.txt')
    $releasePath = Get-BksOutputFile $output ($stem + '-manifest.json')
    $sumPath = Get-BksOutputFile $output ($stem + '-SHA256SUMS.txt')
    Copy-Item -LiteralPath $archive -Destination (Get-BksOutputFile $output ($stem + '-portable.zip')) -Force
    Copy-Item -LiteralPath $setup -Destination (Get-BksOutputFile $output ($stem + '-setup.exe')) -Force
    Copy-Item -LiteralPath (Join-Path $payload 'dependencies.txt') -Destination (Get-BksOutputFile $output ($stem + '-dependencies.txt')) -Force
    $artifacts = @($portableOutput, $setupOutput, $dependenciesOutput)
    $release = [ordered]@{
        schema = 1; product = 'BKS-Term'; version = $Version; architecture = 'x86_64'
        packaged_at_utc = $manifest.packaged_at_utc; executable_sha256 = $binaryHash
        executable_signature = $signatureStatus
        installer_signature = 'unsigned'; installer = '.NET Framework self-extractor / per-user PowerShell 5.1'
        checks = @{ package_file_hashes = 'passed'; installer_extract_only = 'passed'; clean_vm = 'not_run'; human_nvda = 'not_run' }
        artifacts = @($artifacts | ForEach-Object { [pscustomobject]@{
            file = [IO.Path]::GetFileName($_); bytes = (Get-Item -LiteralPath $_).Length; sha256 = Get-BksHash $_
        } })
    }
    $releasePath = Get-BksOutputFile $output ($stem + '-manifest.json')
    Write-BksJson $releasePath $release
    $sumLines = @(@($artifacts) + @($releasePath) | ForEach-Object { (Get-BksHash $_) + '  ' + [IO.Path]::GetFileName($_) })
    $sumPath = Get-BksOutputFile $output ($stem + '-SHA256SUMS.txt')
    [IO.File]::WriteAllLines($sumPath, $sumLines, (New-Object Text.UTF8Encoding $false))
    Write-Output "Portable: $portableOutput"
    Write-Output "Installer: $setupOutput"
    Write-Output "Manifest: $releasePath"
    Write-Output "Checksums: $sumPath"
    Write-Output 'SFX extraction and file hashes passed. Clean-VM and human NVDA acceptance remain manual.'
} finally { Remove-BksPrivateTree $work $output '.bks-package-' }
