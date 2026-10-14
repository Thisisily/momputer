# Compare fresh CLI builds with Defender intact. Never execute the resulting binary,
# change exclusions/settings, restore quarantine, or publish a release.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$SourceDirectory,
    [Parameter(Mandatory)][string]$EvidenceDirectory,
    [Parameter(Mandatory)][ValidateSet('baseline', 'candidate')][string]$Label,
    [Parameter(Mandatory)][string]$RequestedRef
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false # CheckExclusion returns 1 for NOT excluded.
$started = [datetime]::UtcNow
$source = [IO.Path]::GetFullPath($SourceDirectory)
$evidence = [IO.Path]::GetFullPath($EvidenceDirectory)
$binary = [IO.Path]::GetFullPath((Join-Path $source 'target/release/mom.exe'))
$eventLog = 'Microsoft-Windows-Windows Defender/Operational'
$detectionIds = @(1006, 1007, 1008, 1015, 1116, 1117, 1118, 1119)
$protectionIds = @(5001, 5007, 5010, 5012)
$problems = [Collections.Generic.List[string]]::new()
$scanner = $null
$report = [ordered]@{
    result = 'inconclusive'; label = $Label; requestedRef = $RequestedRef; sourceSha = $null
    workflowSha = $env:GITHUB_SHA; runId = $env:GITHUB_RUN_ID; runAttempt = $env:GITHUB_RUN_ATTEMPT
    startedUtc = $started.ToString('o'); runnerImage = $env:ImageOS; runnerImageVersion = $env:ImageVersion
    buildCommand = 'cargo build --release --locked -p mom-cli'; buildExitCode = $null
    binaryExecuted = $false; settingsChanged = $false; scanCompleted = $false
    preflight = $null; afterScan = $null; binaryBefore = $null; binaryAfter = $null
    events = @(); scanEvidence = @(); newDetections = @(); problems = @()
}

function Set-Output([string]$Name, [string]$Value) {
    if ($env:GITHUB_OUTPUT) { Add-Content -LiteralPath $env:GITHUB_OUTPUT -Value "$Name=$Value" }
}
function Protect-Text([string]$Text) {
    foreach ($entry in @(@($source, '<source>'), @($evidence, '<evidence>'),
        @($env:GITHUB_WORKSPACE, '<workspace>'), @($env:USERPROFILE, '<user-profile>'))) {
        if ($entry[0]) { $Text = $Text.Replace([string]$entry[0], [string]$entry[1], [StringComparison]::OrdinalIgnoreCase) }
    }
    return $Text
}
function Get-DefenderState {
    $status = Get-MpComputerStatus -ErrorAction Stop | Select-Object AMRunningMode, AMServiceEnabled,
        AntivirusEnabled, RealTimeProtectionEnabled, BehaviorMonitorEnabled, IoavProtectionEnabled,
        AMProductVersion, AMEngineVersion, AntivirusSignatureVersion, AntivirusSignatureLastUpdated,
        DefenderSignaturesOutOfDate
    $prefs = Get-MpPreference -ErrorAction Stop
    $counts = [ordered]@{}
    foreach ($name in 'ExclusionPath', 'ExclusionExtension', 'ExclusionProcess') {
        $property = $prefs.PSObject.Properties[$name]
        $counts[$name] = if ($null -eq $property) { $null } else { @($property.Value | Where-Object { $_ }).Count }
    }
    return [ordered]@{
        status = $status; exclusionCounts = $counts; exclusions = @()
        preferences = $prefs | Select-Object DisableRealtimeMonitoring, DisableBehaviorMonitoring,
            DisableIOAVProtection, DisableScriptScanning, HideExclusionsFromLocalAdmins
    }
}
function Assert-Protection($State) {
    foreach ($name in 'AMServiceEnabled', 'AntivirusEnabled', 'RealTimeProtectionEnabled', 'BehaviorMonitorEnabled', 'IoavProtectionEnabled') {
        if ($State.status.$name -ne $true) { throw "Defender $name is not confirmed enabled." }
    }
    if ($State.status.AMRunningMode -ne 'Normal') { throw 'Defender is not in normal active mode.' }
    if (-not $State.status.AMEngineVersion -or -not $State.status.AntivirusSignatureVersion -or
        $State.status.DefenderSignaturesOutOfDate -ne $false) { throw 'Defender engine/signatures are missing or stale.' }
    foreach ($name in 'DisableRealtimeMonitoring', 'DisableBehaviorMonitoring', 'DisableIOAVProtection', 'DisableScriptScanning') {
        if ($State.preferences.$name -ne $false) { throw "Defender $name is enabled or unknown." }
    }
    if ($State.preferences.HideExclusionsFromLocalAdmins -ne $false) { throw 'Exclusion visibility is hidden or unknown; coverage cannot be established.' }
    foreach ($name in 'ExclusionPath', 'ExclusionExtension', 'ExclusionProcess') {
        if ($null -eq $State.exclusionCounts[$name]) { throw "Cannot read $name exclusion metadata." }
    }
    # Path checks cannot establish process-opened-file exclusion coverage. Do not guess at
    # wildcard semantics, change policy, or export potentially private exclusion values.
    if ($State.exclusionCounts.ExclusionProcess -ne 0) { throw 'Process exclusions exist; complete build-time coverage cannot be established.' }
}
function Test-Coverage($State) {
    foreach ($path in @($source, (Join-Path $source 'target'), $binary)) {
        $output = @(& $scanner -CheckExclusion -Path $path 2>&1 | ForEach-Object { Protect-Text "$_" })
        $code = $LASTEXITCODE
        $State.exclusions += [ordered]@{ path = Protect-Text $path; exitCode = $code; output = $output }
        # Microsoft documents 0 = excluded; 1 = not excluded; other results are unknown.
        if ($code -ne 1) { throw "Defender coverage not confirmed for $(Protect-Text $path) (exit $code)." }
    }
}
function Get-Events {
    try {
        $entries = @(Get-WinEvent -FilterHashtable @{
            LogName = $eventLog; StartTime = $started; Id = @(1000, 1001, 1002) + $detectionIds + $protectionIds
        } -ErrorAction Stop)
    } catch {
        if ($_.FullyQualifiedErrorId -like 'NoMatchingEventsFound*') { return @() }
        throw
    }
    foreach ($event in $entries) {
        $data = [ordered]@{}
        foreach ($item in ([xml]$event.ToXml()).GetElementsByTagName('Data')) {
            $name = $item.GetAttribute('Name')
            if ($name -notmatch '(?i)user|old value|new value') { $data[$name] = Protect-Text $item.InnerText }
        }
        [ordered]@{ id = $event.Id; recordId = $event.RecordId; timeUtc = $event.TimeCreated.ToUniversalTime().ToString('o'); data = $data }
    }
}
function Get-NewDetections {
    Get-MpThreatDetection -ErrorAction Stop | Where-Object {
        $_.InitialDetectionTime -ge $started -or $_.LastThreatStatusChangeTime -ge $started
    } | Select-Object DetectionID, ThreatID, InitialDetectionTime, LastThreatStatusChangeTime,
        RemediationTime, ActionSuccess, CleaningActionID, ThreatStatusID,
        @{ Name = 'Resources'; Expression = { @($_.Resources | ForEach-Object { Protect-Text "$_" }) } }
}
function Get-BinaryEvidence {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { return @{ exists = $false } }
    return @{
        exists = $true; sha256 = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()
        bytes = (Get-Item -LiteralPath $binary).Length
        signatureStatus = [string](Get-AuthenticodeSignature -LiteralPath $binary).Status
    }
}

Set-Output 'clean' 'false'
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
try {
    if (-not $IsWindows) { throw 'Windows is required.' }
    if ($source -eq $evidence -or $evidence.StartsWith($source + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Evidence must be outside the source checkout.' }
    $report.sourceSha = (& git -C $source rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $report.sourceSha -notmatch '^[0-9a-f]{40}$') { throw 'Cannot resolve the source commit.' }
    Set-Output 'source_sha' $report.sourceSha
    $dirty = @(& git -C $source status --porcelain --untracked-files=all)
    if ($LASTEXITCODE -ne 0 -or $dirty.Count -ne 0 -or (Test-Path -LiteralPath (Join-Path $source 'target'))) { throw 'Use a clean source checkout without an existing target directory or build cache.' }
    $report['cargoLockSha256'] = (Get-FileHash -LiteralPath (Join-Path $source 'Cargo.lock') -Algorithm SHA256).Hash
    $report.preflight = Get-DefenderState
    Assert-Protection $report.preflight
    if (-not (Get-WinEvent -ListLog $eventLog -ErrorAction Stop).IsEnabled) { throw 'Defender event logging is disabled.' }
    $null = @(Get-NewDetections) # Confirm evidence access before compiling.
    $platform = Join-Path $env:ProgramData 'Microsoft/Windows Defender/Platform'
    $candidates = @(Get-ChildItem -LiteralPath $platform -Directory -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending | ForEach-Object { Join-Path $_.FullName 'MpCmdRun.exe' })
    $candidates += Join-Path $env:ProgramFiles 'Windows Defender/MpCmdRun.exe'
    $scannerCandidate = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    if (-not $scannerCandidate) { throw 'Microsoft Defender command-line scanner is unavailable.' }
    $signature = Get-AuthenticodeSignature -LiteralPath $scannerCandidate
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation(?:,|$)') { throw 'The system scanner lacks a valid Microsoft signature.' }
    $scanner = $scannerCandidate
    $report['scannerVersion'] = (Get-Item -LiteralPath $scanner).VersionInfo.FileVersion
    Test-Coverage $report.preflight
    $report['rustc'] = @(& rustc -vV)
    if ($LASTEXITCODE -ne 0 -or $report.rustc -notcontains 'release: 1.99.0' -or $report.rustc -notcontains 'host: x86_64-pc-windows-msvc') { throw 'Rust 1.99.0 for x86_64-pc-windows-msvc is required.' }
    Push-Location $source
    try {
        & cargo build --release --locked -p mom-cli 2>&1 | ForEach-Object { Protect-Text "$_" } | Tee-Object -FilePath (Join-Path $evidence 'build.log')
        $report.buildExitCode = $LASTEXITCODE
    } finally { Pop-Location }
    if ($report.buildExitCode -ne 0) { throw 'Build failed; inspect build and Defender evidence.' }
    if ((Get-FileHash -LiteralPath (Join-Path $source 'Cargo.lock') -Algorithm SHA256).Hash -ne $report.cargoLockSha256) { throw 'Cargo.lock changed during the build.' }
    $report.binaryBefore = Get-BinaryEvidence
    if (-not $report.binaryBefore.exists) { throw 'mom.exe did not survive the build; it will not be restored or uploaded.' }
    $report['scanRequestedUtc'] = [datetime]::UtcNow.ToString('o')
    # Ordinary custom scan, with normal remediation. No DisableRemediation option.
    $job = Start-MpScan -ScanType CustomScan -ScanPath $binary -AsJob -ErrorAction Stop
    $finished = Wait-Job -Job $job -Timeout 300
    if ($null -eq $finished -or $job.State -ne 'Completed') { throw 'Custom scan did not complete within five minutes.' }
    Receive-Job -Job $job -ErrorAction Stop | Out-String | Set-Content -LiteralPath (Join-Path $evidence 'scan.log')
    # Allow event publication to catch up. Require the scan ID for this exact file;
    # an unrelated quick scan or a successful remediation must never count as clean.
    for ($attempt = 0; $attempt -lt 15; $attempt++) {
        $events = @(Get-Events)
        $ids = @($events | Where-Object {
            $_.id -eq 1000 -and $_.timeUtc -ge $report.scanRequestedUtc -and
            $_.data['Scan Resources'] -like '*<source>\target\release\mom.exe*'
        } | ForEach-Object { $_.data['Scan ID'] } | Where-Object { $_ })
        if (@($events | Where-Object { $_.id -eq 1002 -and $_.data['Scan ID'] -in $ids }).Count) { throw 'Defender cancelled the custom scan.' }
        if (@($events | Where-Object { $_.id -eq 1001 -and $_.data['Scan ID'] -in $ids }).Count) {
            $report.scanEvidence = @($events | Where-Object { $_.id -in @(1000, 1001) -and $_.data['Scan ID'] -in $ids })
            $report.scanCompleted = $true
            break
        }
        Start-Sleep -Seconds 2
    }
    if (-not $report.scanCompleted) { throw 'No completed event could be correlated to the requested file scan.' }
} catch { $problems.Add((Protect-Text $_.Exception.Message)) }
finally {
    # This also runs if real-time protection quarantined the binary during compilation.
    try {
        $report.afterScan = Get-DefenderState
        Assert-Protection $report.afterScan
        if (-not $scanner) { throw 'No verified scanner is available for final exclusion checks.' }
        Test-Coverage $report.afterScan
    } catch { $problems.Add((Protect-Text $_.Exception.Message)) }
    try { $report.newDetections = @(Get-NewDetections) } catch { $problems.Add("Threat evidence unavailable: $(Protect-Text $_.Exception.Message)") }
    try { $report.events = @(Get-Events) } catch { $problems.Add("Event evidence unavailable: $(Protect-Text $_.Exception.Message)") }
    try { $report.binaryAfter = Get-BinaryEvidence } catch { $problems.Add("Final file evidence unavailable: $(Protect-Text $_.Exception.Message)") }
    if (@($report.events | Where-Object { $_.id -in $protectionIds }).Count) { $problems.Add('Defender protection/configuration changed during this experiment; review the events.') }
    if ($report.newDetections.Count -or @($report.events | Where-Object { $_.id -in $detectionIds }).Count) {
        $report.result = 'detected'
        $problems.Add('Defender reported detection/remediation during the experiment. No binary will be uploaded; inspect the resources to identify the affected file.')
    } elseif ($problems.Count -eq 0 -and $report.scanCompleted -and $report.binaryBefore -and $report.binaryAfter -and
        $report.binaryBefore.exists -and $report.binaryAfter.exists -and $report.binaryBefore.sha256 -eq $report.binaryAfter.sha256) {
        $report.result = 'clean'
    } elseif ($problems.Count -eq 0) { $problems.Add('A completed scan and byte-identical file survival were not established.') }
    $report['finishedUtc'] = [datetime]::UtcNow.ToString('o')
    $report.problems = @($problems.ToArray())
    $report | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $evidence 'report.json') -Encoding utf8
    $summary = @("Defender diagnostic: $Label — $($report.result)", "Source: $($report.sourceSha)",
        'The CLI was never executed. Defender settings and quarantine were not changed.',
        'Compare the recorded engine/signatures for both runs before comparing verdicts. A clean scan does not prove a previous detection false.') + @($problems.ToArray())
    $summary | Set-Content -LiteralPath (Join-Path $evidence 'summary.txt') -Encoding utf8
    if ($env:GITHUB_STEP_SUMMARY) { $summary | Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY }
    Write-Host ($summary -join "`n")
}
if ($report.result -ne 'clean') { exit 1 }
Set-Output 'clean' 'true'
