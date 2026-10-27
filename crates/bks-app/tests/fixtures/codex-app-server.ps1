param([ValidateSet('stream','interrupt','oversize','silent','stale','blocked','earlytree','leaf')][string]$Mode = 'stream')
# Hermetic fake stdio peer. It never invokes Codex, authenticates, runs tools,
# reads a user profile/configuration, or sends network/model/API requests.
$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = New-Object System.Text.UTF8Encoding($false)
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
function Reply($value) {
    [Console]::Out.WriteLine(($value | ConvertTo-Json -Compress -Depth 12))
    [Console]::Out.Flush()
}
if ($Mode -eq 'leaf') { Start-Sleep -Seconds 60; exit }
$fixtureModel = 'fixture-model'
if ($Mode -eq 'earlytree') {
    # Spawn immediately, before reading even initialize. The transport must
    # assign the parent to its job before any fixture startup code executes.
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
    $start.Arguments = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + $PSCommandPath + '" -Mode leaf'
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $leaf = [Diagnostics.Process]::Start($start)
    $fixtureModel = 'fixture-child-' + $leaf.Id
    $leaf.Dispose()
}
if ($Mode -eq 'silent') {
    Start-Sleep -Seconds 60
    exit
}
if ($Mode -eq 'oversize') {
    [Console]::Out.WriteLine(('x' * (129 * 1024)))
    [Console]::Out.Flush()
    Start-Sleep -Seconds 60
    exit
}
while ($null -ne ($line = [Console]::In.ReadLine())) {
    $request = $line | ConvertFrom-Json
    if ($Mode -eq 'stale' -and $request.id -eq 'fixture-approval' -and $null -ne $request.result) {
        # This branch is a test failure: a stale approval reached the wire.
        Reply @{ id=99999; error=@{ code=-32600; message='FORBIDDEN: stale decision reached fake server' } }
    }
    switch ($request.method) {
        'initialize' {
            Reply @{ id=$request.id; result=@{ codexHome='C:\fixture-home'; userAgent='fixture/0.164.0'; platformFamily='windows'; platformOs='windows' } }
        }
        'thread/start' {
            Reply @{ id=$request.id; result=@{ thread=@{ id='thread-1' }; approvalPolicy='untrusted'; approvalsReviewer='user'; sandbox=@{ type='readOnly'; networkAccess=$false }; cwd='C:\fixture'; model=$fixtureModel } }
            if ($Mode -eq 'blocked' -or $Mode -eq 'earlytree') { Start-Sleep -Seconds 60; exit }
        }
        'turn/start' {
            Reply @{ id=$request.id; result=@{ turn=@{ id='turn-1'; status='inProgress' } } }
            Reply @{ method='turn/started'; params=@{ threadId='thread-1'; turn=@{ id='turn-1'; status='inProgress' } } }
            if ($Mode -eq 'stream') {
                Reply @{ method='item/agentMessage/delta'; params=@{ threadId='thread-1'; turnId='turn-1'; itemId='message-1'; delta='fixture response' } }
                Reply @{ method='item/completed'; params=@{ threadId='thread-1'; turnId='turn-1'; item=@{ type='agentMessage'; id='message-1'; text='fixture response' } } }
                Reply @{ method='turn/completed'; params=@{ threadId='thread-1'; turn=@{ id='turn-1'; status='completed' } } }
            }
            if ($Mode -eq 'stale') {
                Reply @{ id='fixture-approval'; method='item/commandExecution/requestApproval'; params=@{ threadId='thread-1'; turnId='turn-1'; itemId='command-1'; startedAtMs=123; command='Get-ChildItem'; cwd='C:\fixture'; reason='synthetic fixture'; kind='command' } }
                Start-Sleep -Milliseconds 500
                Reply @{ method='serverRequest/resolved'; params=@{ threadId='thread-1'; requestId='fixture-approval' } }
                Reply @{ method='turn/completed'; params=@{ threadId='thread-1'; turn=@{ id='turn-1'; status='completed' } } }
            }
        }
        'turn/interrupt' {
            Reply @{ id=$request.id; result=@{} }
            Reply @{ method='turn/completed'; params=@{ threadId='thread-1'; turn=@{ id='turn-1'; status='interrupted' } } }
        }
    }
}
