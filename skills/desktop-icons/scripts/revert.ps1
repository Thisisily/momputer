# revert.ps1 - undoes a tidy-up recorded in undo-log.csv.
#
# This script is the same for every run and contains no file names. Every path comes
# from the CSV as data and is only ever handed to .NET file calls, never to a PowerShell
# file cmdlet, so a file name can never become part of a command, whatever characters
# it holds.
#
# The CSV is a file anyone could have edited, so each row is checked before anything
# moves: only full paths on a local disk, nothing inside Windows, Program Files,
# ProgramData or AppData, never a drive or one of your main folders itself (Desktop,
# Documents, Downloads...), nothing through a link, never over an existing name. Anything
# that can't be examined is refused, never treated as fine. This script does not know
# which folders a run was limited to, or a skill's own rules: the skill's
# "apply_plan.py --undo" checks those as well.
#
# Run the copy in the skill's scripts folder and point it at the log folder. The copy kept
# next to each log is the same script, there for convenience: without -Log it reads the
# undo-log.csv next to it.
#
#   powershell -ExecutionPolicy Bypass -File revert.ps1 -Log <log folder> -Preview   # only show what would happen
#   powershell -ExecutionPolicy Bypass -File revert.ps1 -Log <log folder>            # revert
#   ... -Root <folder>   # only if the tidy-up was run with --root <folder> on a folder in a protected place
param([switch]$Preview, [string]$Root, [string]$Log)

Set-StrictMode -Version 2
$ErrorActionPreference = 'Stop'

# Looks one name up exactly as it will be opened (no PowerShell path handling in between) and
# says what is really there: its real spelling on disk, whether it is a plain file or folder,
# a cloud placeholder (OneDrive), a link (junction or symlink) or something else, and whether
# it is a folder. A lookup that fails for any reason other than "nothing has that name" says
# "unknown": that is never the same as not being there.
if (-not ('MomTidyProbe2' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class MomTidyProbe2 {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct WIN32_FIND_DATA {
        public uint dwFileAttributes;
        public System.Runtime.InteropServices.ComTypes.FILETIME ftCreationTime;
        public System.Runtime.InteropServices.ComTypes.FILETIME ftLastAccessTime;
        public System.Runtime.InteropServices.ComTypes.FILETIME ftLastWriteTime;
        public uint nFileSizeHigh;
        public uint nFileSizeLow;
        public uint dwReserved0;
        public uint dwReserved1;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string cFileName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 14)] public string cAlternateFileName;
    }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr FindFirstFileW(string lpFileName, out WIN32_FIND_DATA lpFindFileData);
    [DllImport("kernel32.dll")]
    static extern bool FindClose(IntPtr hFindFile);

    // null only if nothing has that name; else
    // { real name, "plain" | "cloud" | "link" | "other" | "unknown", "dir" | "file" | error code }
    public static string[] Look(string extendedPath) {
        WIN32_FIND_DATA d;
        IntPtr h = FindFirstFileW(extendedPath, out d);
        if (h == new IntPtr(-1)) {
            int err = Marshal.GetLastWin32Error();
            if (err == 2 || err == 3) return null;      // file not found, path not found
            return new string[] { "", "unknown", err.ToString() };   // access denied and everything else
        }
        FindClose(h);
        string kind = "plain";
        if ((d.dwFileAttributes & 0x400u) != 0) {          // a reparse point: the tag says which kind
            uint tag = d.dwReserved0;
            if (tag == 0xA0000003u || tag == 0xA000000Cu) kind = "link";        // junction, symlink
            else if ((tag & 0xFFFF0FFFu) == 0x9000001Au) kind = "cloud";        // OneDrive and other cloud files
            else kind = "other";
        }
        return new string[] { d.cFileName, kind, (d.dwFileAttributes & 0x10u) != 0 ? "dir" : "file" };
    }
}
'@
}

function Ext([string]$p) {
    # Extended-length form: the exact name (trailing dots or spaces, long paths), no wildcards
    if ($p.StartsWith('\\?\')) { return $p }
    return '\\?\' + $p
}
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
$rootGiven = ''
if ($Root) { $rootGiven = [System.IO.Path]::GetFullPath($Root); $rootDir = $rootGiven }
$protected = @()
foreach ($v in @($env:SystemRoot, $env:ProgramFiles, ${env:ProgramFiles(x86)}, $env:ProgramData, $env:APPDATA, $env:LOCALAPPDATA)) {
    if ($v) { $protected += $v }
}
if ($env:USERPROFILE) { $protected += [System.IO.Path]::Combine($env:USERPROFILE, 'AppData') }
# A tidy-up run with --root on a test folder under Temp: that folder alone is exempt, never
# the protected place around it. Anywhere else -Root changes nothing.
if ($rootDir) {
    $temp = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
    $holds = @($protected | Where-Object { Under $_ $rootDir }).Count -gt 0
    if ($holds -or -not (Under $rootDir $temp) -or (Clean $rootDir) -eq (Clean $temp)) { $rootDir = '' }
}

# The person's main folders, wherever Windows says they are (one may be redirected, into
# OneDrive say), and the Public ones. A tidy-up never moves one of these folders itself, so a
# row that names one was not written by a tidy-up. Windows and the registry are asked: nothing
# here comes from the log.
$mains = @()
foreach ($f in @('UserProfile', 'DesktopDirectory', 'MyDocuments', 'MyPictures', 'MyMusic', 'MyVideos',
                 'CommonDesktopDirectory', 'CommonDocuments', 'CommonPictures', 'CommonMusic', 'CommonVideos')) {
    $v = ''
    try { $v = [Environment]::GetFolderPath([Environment+SpecialFolder]$f) } catch { $v = '' }
    if ($v) { $mains += $v }
}
$downloads = ''
try {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders')
    if ($key) {
        # The values the Python engine reads. Downloads has no name of its own there, only this id.
        foreach ($n in @('Desktop', 'Personal', 'My Pictures', 'My Music', 'My Video', '{374DE290-123F-4565-9164-39C4925E467B}')) {
            $v = [string]$key.GetValue($n, '')
            if ($v) { $mains += $v }
            if ($v -and $n.StartsWith('{')) { $downloads = $v }
        }
        $key.Close()
    }
} catch { }
foreach ($v in @($env:USERPROFILE, $env:PUBLIC, $env:OneDrive, $env:OneDriveConsumer, $env:OneDriveCommercial)) {
    if ($v) { $mains += $v }
}
if (-not $downloads -and $env:USERPROFILE) { $mains += [System.IO.Path]::Combine($env:USERPROFILE, 'Downloads') }
if ($env:PUBLIC) { $mains += [System.IO.Path]::Combine($env:PUBLIC, 'Downloads') }
# With -Root, that folder stands in for the profile: the same names inside it count too.
if ($rootGiven) {
    $mains += $rootGiven
    foreach ($n in @('Desktop', 'Documents', 'Downloads', 'Pictures', 'Videos', 'Music')) {
        $mains += [System.IO.Path]::Combine($rootGiven, $n)
    }
}

function WhyNot([string]$p) {
    # $null if the path may be touched, else the reason it may not.
    if ($p -match '^[A-Za-z]:\\?$') { return 'a drive root' }
    if ($p -notmatch '^[A-Za-z]:\\[^\\]') { return 'not a full path on a local disk' }
    if ($p.Substring(2).Contains(':')) { return 'a name with a colon in it' }
    if ($p.Contains('/')) { return 'a path with a forward slash' }
    # The path that is checked must be the path that is used: nothing Windows would quietly
    # rewrite or read as another name.
    if ($p -match '[<>"|?*\x00-\x1f]') { return 'a name with characters Windows does not allow' }
    $names = @($p.Substring(3).Split('\'))
    for ($i = 0; $i -lt $names.Count; $i++) {
        $part = $names[$i]
        if ($part -eq '') { return 'a doubled or trailing backslash' }
        if ($part -eq '..' -or $part -eq '.') { return 'a path with dots for a folder' }
        if ($i -lt $names.Count - 1 -and ($part.EndsWith('.') -or $part.EndsWith(' '))) { return 'a folder name ending in a dot or space' }
    }
    try { $kind = (New-Object System.IO.DriveInfo ($p.Substring(0, 3))).DriveType } catch { return 'an unknown drive' }
    if ($kind -ne [System.IO.DriveType]::Fixed -and $kind -ne [System.IO.DriveType]::Removable) { return 'on a network drive, not a local disk' }
    foreach ($main in $mains) {
        if ((Clean $p) -eq (Clean $main)) { return 'one of your main folders' }
    }
    $exempt = $rootDir -and (Under $p $rootDir)
    foreach ($prot in $protected) {
        if ((Under $p $prot) -and -not $exempt) { return 'inside Windows, Program Files, ProgramData or AppData' }
    }
    # Every part that exists is looked up by its exact name, the same way it is opened later.
    # It must be spelled as it really is on disk (a short 8.3 name such as APPDAT~1, or any
    # other spelling Windows accepts for it, is another name for a folder checked above), and
    # it must not be a link: a junction or symlink can lead into a protected folder. A part
    # that can't be looked up (a folder that may be passed through but not listed, say) ends
    # the row: what is behind it could be a link.
    $cur = $p.Substring(0, 3)
    for ($i = 0; $i -lt $names.Count; $i++) {
        $cur = $cur + $names[$i]
        $seen = [MomTidyProbe2]::Look((Ext $cur))
        if ($null -eq $seen) { break }   # nothing by that name: it, and the rest, would be new
        if ([string]$seen[1] -eq 'unknown') { return 'a path that can''t be examined' }
        if (-not [string]::Equals([string]$seen[0], $names[$i], [System.StringComparison]::OrdinalIgnoreCase)) {
            return 'not the real name of that file or folder (a short name or another alias)'
        }
        if ([string]$seen[1] -ne 'plain' -and [string]$seen[1] -ne 'cloud') { return 'a link to another place, or reached through one' }
        if ($i -lt $names.Count - 1 -and [string]$seen[2] -ne 'dir') { return 'a path through something that is not a folder' }
        $cur = $cur + '\'
    }
    return $null
}

# -Log names the folder that holds undo-log.csv (or that file itself). Without it: the folder this script is in.
$csv = [System.IO.Path]::Combine($PSScriptRoot, 'undo-log.csv')
if ($Log) {
    $csv = [System.IO.Path]::GetFullPath($Log)
    if (-not [System.IO.File]::Exists($csv)) { $csv = [System.IO.Path]::Combine($csv, 'undo-log.csv') }
}
if (-not [System.IO.File]::Exists($csv)) {
    if ($Log) { Write-Host 'There is no undo-log.csv in that folder. Nothing to revert.' }
    else { Write-Host 'undo-log.csv is not next to this script. Nothing to revert.' }
    exit 1
}
$text = [System.IO.File]::ReadAllText($csv, [System.Text.Encoding]::UTF8)
$rows = @()
if ($text.Trim()) { $rows = @(ConvertFrom-Csv -InputObject $text) }
[array]::Reverse($rows)

$back = 0; $skipped = 0; $manual = 0; $far = 0; $failed = 0; $refused = 0
foreach ($row in $rows) {
    # Everything about a row happens inside this try: whatever goes wrong, the row ends as
    # SKIP, REFUSED, MANUAL or FAILED, and nothing is moved.
    $from = ''
    try {
        if ([string]$row.status -ne 'DONE') { continue }
        $from = [string]$row.'from'
        $to = [string]$row.'to'
        $op = [string]$row.op
        if ($op -ne 'move' -and $op -ne 'mkdir' -and $op -ne 'remove-empty-dir' -and $op -ne 'recycle') { continue }
        $why = WhyNot $from
        if (-not $why -and $op -eq 'move') {
            $why = WhyNot $to
        }
        if ($why) { Say 'REFUSED' $from $why; $refused++; continue }
        if ($op -eq 'move') {
            $fromX = Ext $from
            $toX = Ext $to
            $there = [MomTidyProbe2]::Look($toX)
            if ($null -eq $there) { Say 'SKIP' $to 'no longer there'; $skipped++; continue }
            if ($null -ne [MomTidyProbe2]::Look($fromX)) { Say 'SKIP' $from 'that name is taken again'; $skipped++; continue }
            if ([string]$there[1] -ne 'plain' -and [string]$there[1] -ne 'cloud') { Say 'REFUSED' $from 'what was moved can''t be examined, or is a link'; $refused++; continue }
            $isDir = ([string]$there[2] -eq 'dir')
            if ($isDir -and $from.Substring(0, 1).ToUpperInvariant() -ne $to.Substring(0, 1).ToUpperInvariant()) {
                # Bringing a folder back from another drive means copying what is inside it. That is
                # left to the Python undo, which checks the folder for links first.
                Say 'MANUAL' $from 'a folder on another drive: use apply_plan.py --undo'
                $far++
                continue
            }
            if (-not $Preview) {
                $parent = $fromX.Substring(0, $fromX.LastIndexOf('\'))
                if ($parent.EndsWith(':')) { $parent = $parent + '\' }   # the drive itself
                if (-not [System.IO.Directory]::Exists($parent)) { [void][System.IO.Directory]::CreateDirectory($parent) }
                if ($isDir) { [System.IO.Directory]::Move($toX, $fromX) }
                else { [System.IO.File]::Move($toX, $fromX) }
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
if ($far -gt 0) {
    Write-Host ('{0} folder(s) are on another drive and were left there: run the skill''s apply_plan.py with --undo and this folder.' -f $far)
}
if ($failed -gt 0 -or $refused -gt 0) { exit 1 }
exit 0
