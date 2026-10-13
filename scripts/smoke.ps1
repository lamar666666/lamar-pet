param([Parameter(Mandatory=$true)][string]$Executable)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$executablePath = (Resolve-Path -LiteralPath $Executable).Path
$testRoot = Join-Path $projectRoot ('.runtime\smoke-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot -Force | Out-Null
foreach ($kind in @('idle','focus','break')) {
    $dataRoot = Join-Path $testRoot $kind
    New-Item -ItemType Directory -Path $dataRoot -Force | Out-Null
    $day = Get-Date -Format 'yyyy-MM-dd'
    $phase = switch ($kind) { 'focus' { 'paused_focus' }; 'break' { 'paused_break' }; default { 'idle' } }
    $checkpoint = @{
        focus_minutes=25; break_minutes=1; phase=$phase
        remaining_millis=$(if($kind -eq 'idle'){0}else{1500})
        calendar_day=$day
        history=@(@{day=$day;focus_millis=1498500;completed_rounds=$(if($kind -eq 'break'){1}else{0})})
        reminder_sequence=$(if($kind -eq 'break'){1}else{0})
        reminder=$(if($kind -eq 'break'){'focus_completed'}else{'none'})
    }
    $document = @{schema_version=1;preferences=@{scale_percent=100;appearance='light'};checkpoint=$checkpoint}
    $json = $document | ConvertTo-Json -Depth 6
    [IO.File]::WriteAllText((Join-Path $dataRoot 'focus.json'),$json,[Text.UTF8Encoding]::new($false))
    $arguments = @('--smoke-seconds','5','--data-dir',$dataRoot)
    if($kind -ne 'idle'){$arguments += '--smoke-resume'}
    $quotedArguments = $arguments | ForEach-Object { '"' + $_ + '"' }
    $process = Start-Process -FilePath $executablePath -ArgumentList $quotedArguments -WindowStyle Hidden -PassThru -Wait
    if($process.ExitCode -ne 0){throw "$kind smoke process failed: $($process.ExitCode)"}
    $report = Get-Content -LiteralPath (Join-Path $dataRoot 'smoke-report.json') -Raw | ConvertFrom-Json
    if($report.errors.Count -gt 0 -or $report.storage_error -or $report.load_pending -or $report.command_error){throw "$kind smoke reported an error"}
    if($report.panel_frames -eq 0 -or $report.pet_frames -eq 0){throw "$kind windows did not render"}
    if($kind -ne 'idle' -and $report.reminder_frames -eq 0){throw "$kind reminder did not render"}
    if($kind -eq 'focus' -and ($report.rounds -ne 1 -or $report.phase -ne 'PausedBreak')){throw 'Focus completion did not persist correctly'}
    if($kind -eq 'break' -and ($report.phase -ne 'Idle' -or $report.reminder_sequence -ne 2)){throw 'Break completion did not persist correctly'}
    $saved = Get-Content -LiteralPath (Join-Path $dataRoot 'focus.json') -Raw | ConvertFrom-Json
    if($saved.schema_version -ne 1){throw "$kind saved document has wrong schema"}
    $today = @($saved.checkpoint.history | Where-Object { $_.day -eq $day })
    if($today.Count -ne 1){throw "$kind daily statistics are missing"}
    if($kind -eq 'focus' -and ($saved.checkpoint.phase -ne 'paused_break' -or $saved.checkpoint.reminder_sequence -ne 1 -or $today[0].completed_rounds -ne 1 -or $saved.checkpoint.remaining_millis -le 0 -or $saved.checkpoint.remaining_millis -gt 60000)){throw 'Saved focus completion is incorrect'}
    if($kind -eq 'break' -and ($saved.checkpoint.phase -ne 'idle' -or $saved.checkpoint.reminder_sequence -ne 2 -or $today[0].completed_rounds -ne 1 -or $saved.checkpoint.remaining_millis -ne 0)){throw 'Saved break completion is incorrect'}
    [pscustomobject]@{Case=$kind;PanelFrames=$report.panel_frames;PetFrames=$report.pet_frames;ReminderFrames=$report.reminder_frames;Phase=$report.phase}
}
Write-Output "Smoke reports: $testRoot"
