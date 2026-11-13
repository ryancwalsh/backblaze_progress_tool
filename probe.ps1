#Requires -RunAsAdministrator
<#
  probe.ps1 - prints the STRUCTURE of Backblaze's local data files with every path,
  file name, e-mail, user name and computer name replaced by placeholders.
  It only reads files. Output goes to probe_output.txt next to this script.
  Review that file before pasting it anywhere.
#>
param(
  [string]$BzData = "$env:ProgramData\Backblaze\bzdata",
  [string]$Out    = (Join-Path $PSScriptRoot 'probe_output.txt')
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
  # anything else that looks like a file name (keep Backblaze's own bz* names and version numbers)
  $s = [regex]::Replace($s, '[^\s"<>|\\/=,;:()\[\]]+\.[A-Za-z0-9]{1,6}\b', {
        param($m) if ($m.Value -match '^(bz|\d|https?)' ) { $m.Value } else { '<NAME>' } })
  if ($s.Length -gt 400) { $s = $s.Substring(0, 400) + ' ...<truncated>' }
  $s
}

function FieldType([string]$f) {
  if ($f -eq '')                        { 'empty' }
  elseif ($f -match $pathRx)            { 'PATH' }
  elseif ($f -match '^-?\d+$')          { "int(len $($f.Length))" }
  elseif ($f -match '^[0-9a-fA-F]{12,}$') { "hex(len $($f.Length))" }
  elseif ($f.Length -le 3)              { "short:'$f'" }
  else                                  { "text(len $($f.Length))" }
}

function Describe-File([IO.FileInfo]$fi, [int]$head = 8, [int]$tail = 3) {
  Emit "---- FILE: $(Redact $fi.Name)   size=$($fi.Length)   modified=$($fi.LastWriteTime.ToString('s'))"
  try {
    $fs = [IO.File]::Open($fi.FullName, 'Open', 'Read', 'ReadWrite')
    $buf = New-Object byte[] 4; [void]$fs.Read($buf, 0, 4); $fs.Close()
    Emit ("first bytes (hex): " + (($buf | ForEach-Object { $_.ToString('X2') }) -join ' '))
    $sr = New-Object IO.StreamReader($fi.FullName, [Text.Encoding]::UTF8, $true)
    $lines = New-Object System.Collections.Generic.List[string]
    $count = 0; $fieldHist = @{}; $posTypes = @{}
    $lastLines = New-Object System.Collections.Generic.Queue[string]
    while ($null -ne ($l = $sr.ReadLine())) {
      $count++
      if ($lines.Count -lt $head) { $lines.Add($l) }
      $lastLines.Enqueue($l); if ($lastLines.Count -gt $tail) { [void]$lastLines.Dequeue() }
      if ($count -le 500) {
        $parts = $l -split "`t"
        $fieldHist[$parts.Count] = 1 + [int]$fieldHist[$parts.Count]
        for ($i = 0; $i -lt $parts.Count; $i++) {
          $k = "$i|$(FieldType $parts[$i])"; $posTypes[$k] = 1 + [int]$posTypes[$k]
        }
      }
    }
    Emit "encoding detected: $($sr.CurrentEncoding.WebName)   lines: $count"; $sr.Close()
    Emit ("tab-field counts (first 500 lines): " + (($fieldHist.GetEnumerator() | Sort-Object Name | ForEach-Object { "$($_.Name) fields x$($_.Value)" }) -join ', '))
    Emit "field position -> types seen (first 500 lines):"
    $posTypes.GetEnumerator() | Sort-Object @{ e = { [int]($_.Name -split '\|')[0] }; Descending = $false }, @{ e = 'Value'; Descending = $true } |
      ForEach-Object { $p = $_.Name -split '\|', 2; Emit ("   [{0}] {1}  x{2}" -f $p[0], $p[1], $_.Value) }
    Emit "first $head lines (redacted, tabs shown as ' | '):"
    foreach ($x in $lines) { Emit ("   " + (Redact ($x -replace "`t", ' | '))) }
    if ($count -gt $head) {
      Emit "last $tail lines (redacted):"
      foreach ($x in $lastLines) { Emit ("   " + (Redact ($x -replace "`t", ' | '))) }
    }
  } catch { Emit "   (could not read: $($_.Exception.Message))" }
  Emit
}

Emit "=== Backblaze probe  $(Get-Date -Format s) ==="
Emit "PowerShell $($PSVersionTable.PSVersion)   OS $([Environment]::OSVersion.VersionString)"
foreach ($exe in @("${env:ProgramFiles(x86)}\Backblaze\bzbui.exe", "$env:ProgramFiles\Backblaze\bzbui.exe")) {
  if (Test-Path $exe) { Emit "Backblaze client version: $((Get-Item $exe).VersionInfo.FileVersion)" }
}
if (-not (Test-Path $BzData)) { Emit "bzdata not found at expected location"; $sb.ToString() | Set-Content $Out -Encoding UTF8; return }

# 1. Inventory, with digits collapsed so daily files group together
Emit; Emit "=== 1. Inventory of bzdata (digits shown as #) ==="
Get-ChildItem $BzData -Recurse -File -Force -ErrorAction SilentlyContinue |
  ForEach-Object {
    $rel = $_.FullName.Substring($BzData.Length).TrimStart('\')
    [pscustomobject]@{ Pattern = (Redact ($rel -replace '\d', '#')); Size = $_.Length; Time = $_.LastWriteTime }
  } |
  Group-Object Pattern | Sort-Object Name |
  ForEach-Object {
    Emit ("{0,-70} n={1,-5} totalKB={2,-10} newest={3}" -f $_.Name, $_.Count,
       [math]::Round(($_.Group | Measure-Object Size -Sum).Sum / 1KB),
       ($_.Group | Sort-Object Time -Descending | Select-Object -First 1).Time.ToString('s'))
  }

# 2. Key backup-state files (newest of each)
Emit; Emit "=== 2. Backup state files ==="
$dc = Join-Path $BzData 'bzbackup\bzdatacenter'
foreach ($pat in 'bz_todo_*', 'bz_done_*') {
  $f = Get-ChildItem $dc -Filter $pat -File -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
  if ($f) { Describe-File $f 10 3 } else { Emit "no $pat in bzdatacenter" }
}

# 3. Logs: newest file in each log folder, plus problem-looking lines
Emit; Emit "=== 3. Logs ==="
$logRoot = Join-Path $BzData 'bzlogs'
Get-ChildItem $logRoot -Directory -ErrorAction SilentlyContinue | ForEach-Object {
  $f = Get-ChildItem $_.FullName -File | Sort-Object LastWriteTime -Descending | Select-Object -First 1
  if ($f) { Emit "## log folder: $($_.Name)"; Describe-File $f 12 6 }
}
Emit "## problem-looking log lines, last 3 days (max 60, redacted):"
$kw = 'error|fail|lock|skip|retry|cannot|can''t|denied|too long|not found|in use|sharing|exclud|issue|warn'
$n = 0
Get-ChildItem $logRoot -Recurse -File -ErrorAction SilentlyContinue |
  Where-Object LastWriteTime -gt (Get-Date).AddDays(-3) | Sort-Object LastWriteTime -Descending |
  ForEach-Object {
    if ($n -ge 60) { return }
    $name = Redact $_.Name
    foreach ($l in [IO.File]::ReadLines($_.FullName)) {
      if ($l -match $kw) { Emit ("   [$name] " + (Redact $l)); $n++; if ($n -ge 60) { break } }
    }
  }

# 4. XML/report/issue files
Emit; Emit "=== 4. Settings, reports and issue files (first 40 lines each, redacted) ==="
Get-ChildItem $BzData -Recurse -File -Force -ErrorAction SilentlyContinue |
  Where-Object { $_.Extension -eq '.xml' -or $_.Name -match 'issue|problem|report|stat' -or $_.DirectoryName -match 'bzreports' } |
  Where-Object { $_.Length -lt 20MB } |
  Sort-Object LastWriteTime -Descending | Select-Object -First 15 |
  ForEach-Object {
    Emit "---- FILE: $(Redact ($_.FullName.Substring($BzData.Length)))   size=$($_.Length)   modified=$($_.LastWriteTime.ToString('s'))"
    try { Get-Content -LiteralPath $_.FullName -TotalCount 40 | ForEach-Object { Emit ("   " + (Redact $_)) } }
    catch { Emit "   (could not read: $($_.Exception.Message))" }
    Emit
  }

$sb.ToString() | Set-Content -LiteralPath $Out -Encoding UTF8
Write-Host "Done. Review, then paste the contents of: $Out"
