# revert.ps1 - undoes a tidy-up recorded in undo-log.csv (the file next to this script).
#
# This script is the same for every run and contains no file names. Every path comes
# from the CSV as data and is only ever handed to .NET file calls or -LiteralPath, so a
# file name can never become part of a command, whatever characters it holds.
#
#   powershell -ExecutionPolicy Bypass -File revert.ps1            # revert
#   powershell -ExecutionPolicy Bypass -File revert.ps1 -Preview   # only show what would happen
param([switch]$Preview)

Set-StrictMode -Version 2
$ErrorActionPreference = 'Stop'

function Ext([string]$p) {
    # Extended-length form: the exact name (trailing dots or spaces, long paths), no wildcards
    if ($p.StartsWith('\\?\')) { return $p }
    if ($p -match '^[A-Za-z]:\\') { return '\\?\' + $p }
    if ($p.StartsWith('\\')) { return '\\?\UNC\' + $p.Substring(2) }
    return $p
}
function IsAbsolute([string]$p) { return ($p -match '^[A-Za-z]:\\') -or $p.StartsWith('\\') }
function Exists([string]$x) { return [System.IO.File]::Exists($x) -or [System.IO.Directory]::Exists($x) }
function Say([string]$what, [string]$path, [string]$note) {
    $line = '{0,-9}{1}' -f $what, $path
    if ($note) { $line = $line + '  (' + $note + ')' }
    Write-Host $line
}

$log = Join-Path $PSScriptRoot 'undo-log.csv'
if (-not (Test-Path -LiteralPath $log)) {
    Write-Host 'undo-log.csv is not next to this script. Nothing to revert.'
    exit 1
}
$rows = @(Import-Csv -LiteralPath $log -Encoding UTF8)
[array]::Reverse($rows)

$back = 0; $skipped = 0; $manual = 0; $failed = 0
foreach ($row in $rows) {
    if ($row.status -ne 'DONE') { continue }
    $from = [string]$row.'from'
    $to = [string]$row.'to'
    $op = [string]$row.op
    if (-not (IsAbsolute $from)) { continue }
    try {
        if ($op -eq 'move') {
            if (-not (IsAbsolute $to)) { continue }
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
Write-Host ('{0}: {1}. Skipped: {2}. Failed: {3}.' -f $verb, $back, $skipped, $failed)
if ($manual -gt 0) {
    Write-Host ('{0} item(s) are in the Recycle Bin: open it, select them and choose Restore.' -f $manual)
}
if ($failed -gt 0) { exit 1 }
exit 0
