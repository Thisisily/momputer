# revert.ps1 - undoes a tidy-up recorded in undo-log.csv (the file next to this script).
#
# This script is the same for every run and contains no file names. Every path comes
# from the CSV as data and is only ever handed to .NET file calls or -LiteralPath, so a
# file name can never become part of a command, whatever characters it holds.
#
# The CSV is a file anyone could have edited, so each row is checked before anything
# moves: only full paths on a local disk, nothing inside Windows, Program Files,
# ProgramData or AppData, nothing through a link, never over an existing name.
#
#   powershell -ExecutionPolicy Bypass -File revert.ps1            # revert
#   powershell -ExecutionPolicy Bypass -File revert.ps1 -Preview   # only show what would happen
#   ... -Root <folder>   # only if the tidy-up was run with --root <folder> on a folder in a protected place
param([switch]$Preview, [string]$Root)

Set-StrictMode -Version 2
$ErrorActionPreference = 'Stop'

function Ext([string]$p) {
    # Extended-length form: the exact name (trailing dots or spaces, long paths), no wildcards
    if ($p.StartsWith('\\?\')) { return $p }
    return '\\?\' + $p
}
function Exists([string]$x) { return [System.IO.File]::Exists($x) -or [System.IO.Directory]::Exists($x) }
function Say([string]$what, [string]$path, [string]$note) {
    $line = '{0,-9}{1}' -f $what, $path
    if ($note) { $line = $line + '  (' + $note + ')' }
    Write-Host $line
}
function Clean([string]$p) { return $p.TrimEnd('\').ToLowerInvariant() }
function Under([string]$p, [string]$root) {
    if (-not $root) { return $false }
    $r = Clean $root
    $q = Clean $p
    return ($q -eq $r) -or $q.StartsWith($r + '\')
}

$rootDir = ''
if ($Root) { $rootDir = [System.IO.Path]::GetFullPath($Root) }
$protected = @()
foreach ($v in @($env:SystemRoot, $env:ProgramFiles, ${env:ProgramFiles(x86)}, $env:ProgramData, $env:APPDATA, $env:LOCALAPPDATA)) {
    if ($v) { $protected += $v }
}
if ($env:USERPROFILE) { $protected += (Join-Path $env:USERPROFILE 'AppData') }
# A tidy-up run with --root on a test folder under Temp: that folder alone is exempt, never
# the protected place around it. Anywhere else -Root changes nothing.
if ($rootDir) {
    $temp = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
    $holds = @($protected | Where-Object { Under $_ $rootDir }).Count -gt 0
    if ($holds -or -not (Under $rootDir $temp) -or (Clean $rootDir) -eq (Clean $temp)) { $rootDir = '' }
}

function WhyNot([string]$p) {
    # $null if the path may be touched, else the reason it may not.
    if ($p -notmatch '^[A-Za-z]:\\[^\\]') { return 'not a full path on a local disk' }
    if ($p.Substring(2).Contains(':')) { return 'a name with a colon in it' }
    if ($p.Contains('/')) { return 'a path with a forward slash' }
    foreach ($part in $p.Split('\')) {
        if ($part -eq '..' -or $part -eq '.') { return 'a path with dots for a folder' }
        if ($part -match '~\d') { return 'a short (8.3) name; use apply_plan.py --undo instead' }
    }
    try { $kind = (New-Object System.IO.DriveInfo ($p.Substring(0, 3))).DriveType } catch { return 'an unknown drive' }
    if ($kind -ne [System.IO.DriveType]::Fixed -and $kind -ne [System.IO.DriveType]::Removable) { return 'on a network drive, not a local disk' }
    $exempt = $rootDir -and (Under $p $rootDir)
    foreach ($prot in $protected) {
        if ((Under $p $prot) -and -not $exempt) { return 'inside Windows, Program Files, ProgramData or AppData' }
    }
    # No folder on the way may be a link (a junction can be another name for a protected folder).
    $cur = $p.Substring(0, 3)
    $parts = @($p.Substring(3).Split('\'))
    for ($i = 0; $i -lt $parts.Count - 1; $i++) {
        $cur = $cur + $parts[$i]
        $x = Ext $cur
        if ([System.IO.Directory]::Exists($x)) {
            $a = [System.IO.File]::GetAttributes($x)
            if (($a -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                $item = Get-Item -LiteralPath $cur -Force -ErrorAction SilentlyContinue
                if (-not $item -or $item.LinkType) { return 'reached through a link to another folder' }
            }
        }
        $cur = $cur + '\'
    }
    return $null
}
function IsLink([string]$p) {
    $x = Ext $p
    if (-not (Exists $x)) { return $false }
    $a = [System.IO.File]::GetAttributes($x)
    if (($a -band [System.IO.FileAttributes]::ReparsePoint) -eq 0) { return $false }
    $item = Get-Item -LiteralPath $p -Force -ErrorAction SilentlyContinue
    return (-not $item) -or [bool]$item.LinkType
}

$log = Join-Path $PSScriptRoot 'undo-log.csv'
if (-not (Test-Path -LiteralPath $log)) {
    Write-Host 'undo-log.csv is not next to this script. Nothing to revert.'
    exit 1
}
$rows = @(Import-Csv -LiteralPath $log -Encoding UTF8)
[array]::Reverse($rows)

$back = 0; $skipped = 0; $manual = 0; $failed = 0; $refused = 0
foreach ($row in $rows) {
    if ($row.status -ne 'DONE') { continue }
    $from = [string]$row.'from'
    $to = [string]$row.'to'
    $op = [string]$row.op
    if ($op -ne 'move' -and $op -ne 'mkdir' -and $op -ne 'remove-empty-dir' -and $op -ne 'recycle') { continue }
    try {
        $why = WhyNot $from
        if (-not $why -and $op -eq 'move') {
            $why = WhyNot $to
            if (-not $why -and (IsLink $to)) { $why = 'a link to another place' }
        }
        if ($why) { Say 'REFUSED' $from $why; $refused++; continue }
        if ($op -eq 'move') {
            $fromX = Ext $from
            $toX = Ext $to
            if (-not (Exists $toX)) { Say 'SKIP' $to 'no longer there'; $skipped++; continue }
            if (Exists $fromX) { Say 'SKIP' $from 'that name is taken again'; $skipped++; continue }
            if (-not $Preview) {
                $parent = $fromX.Substring(0, $fromX.LastIndexOf('\'))
                [void][System.IO.Directory]::CreateDirectory($parent)
                if ([System.IO.Directory]::Exists($toX)) {
                    try { [System.IO.Directory]::Move($toX, $fromX) }
                    catch [System.IO.IOException] { Move-Item -LiteralPath $toX -Destination $fromX }
                } else {
                    [System.IO.File]::Move($toX, $fromX)
                }
            }
            Say 'BACK' $from ''
            $back++
        }
        elseif ($op -eq 'mkdir') {
            $x = Ext $from
            if ([System.IO.Directory]::Exists($x) -and @([System.IO.Directory]::GetFileSystemEntries($x)).Count -eq 0) {
                if (-not $Preview) { [System.IO.Directory]::Delete($x) }   # non-recursive: only ever an empty folder
                Say 'REMOVED' $from 'folder the tidy-up created'
            }
        }
        elseif ($op -eq 'remove-empty-dir') {
            if (-not $Preview) { [void][System.IO.Directory]::CreateDirectory((Ext $from)) }
            Say 'REMADE' $from 'empty folder'
        }
        elseif ($op -eq 'recycle') {
            Say 'MANUAL' $from 'in the Recycle Bin'
            $manual++
        }
    }
    catch {
        Say 'FAILED' $from $_.Exception.Message
        $failed++
    }
}

Write-Host ''
$verb = 'Moved back'
if ($Preview) { $verb = 'Would move back' }
Write-Host ('{0}: {1}. Skipped: {2}. Refused: {3}. Failed: {4}.' -f $verb, $back, $skipped, $refused, $failed)
if ($manual -gt 0) {
    Write-Host ('{0} item(s) are in the Recycle Bin: open it, select them and choose Restore.' -f $manual)
}
if ($failed -gt 0 -or $refused -gt 0) { exit 1 }
exit 0
