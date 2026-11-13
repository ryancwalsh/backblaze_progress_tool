#Requires -RunAsAdministrator
<#
  probe2.ps1 - second structure probe. Read-only. Paths, user file names, e-mail,
  user name and computer name are replaced by placeholders.
  Output: probe2_output.txt next to this script. Review before sharing.
#>
param(
  [string]$BzData = "$env:ProgramData\Backblaze\bzdata",
  [string]$Out    = (Join-Path $PSScriptRoot 'probe2_output.txt')
)

$pathRx = '([A-Za-z]:[\\/][^\t\r\n"<>|]*|\\\\[^\t\r\n"<>|]+)'
$sb = New-Object System.Text.StringBuilder
function Emit([string]$s = '') { [void]$sb.AppendLine($s) }

function Redact([string]$s) {
  if ($null -eq $s) { return '' }
  $s = [regex]::Replace($s, $pathRx, '<PATH>')
  $s = [regex]::Replace($s, '[\w.+-]+@[\w-]+\.[\w.]+', '<EMAIL>')
  foreach ($w in @($env:USERNAME, $env:COMPUTERNAME)) {
    if ($w) { $s = [regex]::Replace($s, [regex]::Escape($w), '<ID>', 'IgnoreCase') }
  }
  $s = [regex]::Replace($s, '[^\s"<>|\\/=,;:()\[\]]+\.[A-Za-z0-9]{1,6}\b', {
        param($m) if ($m.Value -match '^(bz|v0|\d|https?)') { $m.Value } else { '<NAME>' } })
  if ($s.Length -gt 400) { $s = $s.Substring(0, 400) + ' ...<truncated>' }
  $s
}

# Turns a log line into its "shape" so identical messages about different files group together
function Shape([string]$l) {
  $s = $l -replace '^\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\s+\d+\s+-\s*', ''
  $s = Redact $s
  $s = $s -replace '\b[0-9a-fA-F]{12,}\b', '<HEX>'
  $s = $s -replace '\d+', '#'
  if ($s.Length -gt 220) { $s = $s.Substring(0, 220) + '...' }
  $s
}

function Show-Head([string]$rel, [int]$n = 25) {
  $p = Join-Path $BzData $rel
  $f = Get-Item -LiteralPath $p -ErrorAction SilentlyContinue
  if (-not $f) { Emit "---- $rel : not found"; Emit; return }
  $lines = 0; try { $lines = ([IO.File]::ReadLines($f.FullName) | Measure-Object).Count } catch {}
  Emit "---- $rel   size=$($f.Length)   modified=$($f.LastWriteTime.ToString('s'))   lines=$lines"
  try {
    $i = 0
    foreach ($l in [IO.File]::ReadLines($f.FullName)) {
      Emit ("   " + (Redact ($l -replace "`t", ' | '))); $i++; if ($i -ge $n) { break }
    }
  } catch { Emit "   (could not read: $($_.Exception.Message))" }
  Emit
}

Emit "=== Backblaze probe 2  $(Get-Date -Format s) ==="

# 1. Report and state files not shown in probe 1
Emit; Emit "=== 1. Remaining / skipped / problem lists ==="
foreach ($rel in 'bzreports\bzlist_filesremaining.txt', 'bzreports\bzlist_skipped_files.txt',
                 'bzreports\bzlist_skipped_files.txt.old', 'bzreports\bzlargefile_requirescopy_live.dat',
                 'bzreports\bzlargefile_requirescopy.dat', 'bzreports\bzstat_last_bz_todo_error.txt',
                 'bzreports\bzlist_bzdone_retransmit.txt', 'bzreports\bzstat_totalbackup.xml',
                 'bzreports\bzstat_lastbackupcompleted.xml', 'bzreports\bzstat_upload_success.xml',
                 'bzreports\bzdc_filecheck.xml', 'bzreports\bzdefcon.xml',
                 'bzbackup\bzdatacenter\bzcurrentlargefile\bz_todo_for_chunks.dat',
                 'bzbackup\bzdatacenter\bzcurrentlargefile\bz_done_for_chunks.dat',
                 'bzvolumes.xml') {
  Show-Head $rel 25
}
$tofix = Get-ChildItem (Join-Path $BzData 'bzbackup\bzdatacenter\bz_tofix') -File -ErrorAction SilentlyContinue |
         Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($tofix) { Show-Head ($tofix.FullName.Substring($BzData.Length).TrimStart('\')) 10 }

Emit "=== 2. Exclusion rule files (first 40 lines) ==="
Show-Head 'bzexcluderules_editable.xml' 40
Show-Head 'bzexcluderules_mandatory.xml' 40

Emit "=== 3. bzfilelists folder ==="
Get-ChildItem (Join-Path $BzData 'bzfilelists') -File -ErrorAction SilentlyContinue | Sort-Object Length -Descending |
  ForEach-Object { Emit ("   {0,-60} size={1,-12} modified={2}" -f (Redact $_.Name), $_.Length, $_.LastWriteTime.ToString('s')) }
$big = Get-ChildItem (Join-Path $BzData 'bzfilelists') -File -ErrorAction SilentlyContinue |
       Where-Object Extension -ne '.lck' | Sort-Object Length -Descending | Select-Object -First 1
if ($big) { Emit; Show-Head ($big.FullName.Substring($BzData.Length).TrimStart('\')) 12 }

# 4. Value distributions in to-do and today's done files
Emit "=== 4. Field value distributions ==="
$dc = Join-Path $BzData 'bzbackup\bzdatacenter'
$todo = Get-ChildItem $dc -Filter 'bz_todo_*.dat' -File | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($todo) {
  $c0 = @{}; $c1 = @{}; $n = 0; $sum = [int64]0; $max = [int64]0
  foreach ($l in [IO.File]::ReadLines($todo.FullName)) {
    $p = $l -split "`t"; if ($p.Count -lt 6) { continue }
    $n++; $c0[$p[0]] = 1 + [int]$c0[$p[0]]; $c1[$p[1]] = 1 + [int]$c1[$p[1]]
    $v = [int64]0; if ([int64]::TryParse($p[4], [ref]$v)) { $sum += $v; if ($v -gt $max) { $max = $v } }
  }
  Emit "bz_todo $($todo.Name): lines=$n  field4 sum=$sum  max=$max"
  Emit ("   field0 values: " + (($c0.GetEnumerator() | ForEach-Object { "'$($_.Name)' x$($_.Value)" }) -join ', '))
  Emit ("   field1 values: " + (($c1.GetEnumerator() | ForEach-Object { "'$($_.Name)' x$($_.Value)" }) -join ', '))
}
$done = Get-ChildItem $dc -Filter 'bz_done_*.dat' -File | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($done) {
  $d = @{ 1 = @{}; 2 = @{}; 5 = @{}; 7 = @{}; 11 = @{} }; $n = 0
  foreach ($l in [IO.File]::ReadLines($done.FullName)) {
    $p = $l -split "`t"; if ($p.Count -lt 14) { continue }; $n++
    foreach ($k in 1, 2, 5) { $d[$k][$p[$k]] = 1 + [int]$d[$k][$p[$k]] }
    $k7 = $p[7] -replace '_n.*', ''; $d[7][$k7] = 1 + [int]$d[7][$k7]
    $k11 = if ($p[11] -eq '-') { '-' } else { $p[11].Substring(0, [math]::Min(2, $p[11].Length)) + '...' }
    $d[11][$k11] = 1 + [int]$d[11][$k11]
  }
  Emit "bz_done $($done.Name): lines=$n"
  foreach ($k in 1, 2, 5, 7, 11) {
    Emit ("   field$k values: " + (($d[$k].GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 12 |
          ForEach-Object { "'$($_.Name)' x$($_.Value)" }) -join ', '))
  }
  Emit "   sample '!' and '+' lines with k>0 (redacted):"
  $i = 0
  foreach ($l in [IO.File]::ReadLines($done.FullName)) {
    $p = $l -split "`t"; if ($p.Count -lt 14) { continue }
    if ($p[1] -eq '!' -or $p[7] -notmatch '^k0_') { Emit ("      " + (Redact ($l -replace "`t", ' | '))); $i++; if ($i -ge 6) { break } }
  }
}

# 5. Message shapes across all current transmit/filelist/serv logs
Emit; Emit "=== 5. Log message shapes (counts), problem-ish only ==="
$kw = 'WARN|ERROR|skip|fail|lock|cannot|can''t|denied|too long|not found|in use|sharing|exclud|retry|heal|purge|abort|toobig|too big|timeout'
foreach ($folder in 'bztransmit', 'bzfilelist', 'bzserv') {
  $shapes = @{}; $first = @{}; $last = @{}
  Get-ChildItem (Join-Path $BzData "bzlogs\$folder") -Filter *.log -File -ErrorAction SilentlyContinue | Sort-Object LastWriteTime |
    ForEach-Object {
      foreach ($l in [IO.File]::ReadLines($_.FullName)) {
        if ($l -notmatch $kw) { continue }
        $s = Shape $l; $shapes[$s] = 1 + [int]$shapes[$s]
        $ts = if ($l.Length -ge 19) { $l.Substring(0, 19) } else { '' }
        if (-not $first.ContainsKey($s)) { $first[$s] = $ts }; $last[$s] = $ts
      }
    }
  Emit "## $folder"
  $shapes.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 40 |
    ForEach-Object { Emit ("   x{0,-6} [{1} .. {2}]  {3}" -f $_.Value, $first[$_.Name], $last[$_.Name], $_.Name) }
  Emit
}

$sb.ToString() | Set-Content -LiteralPath $Out -Encoding UTF8
Write-Host "Done. Output: $Out"
