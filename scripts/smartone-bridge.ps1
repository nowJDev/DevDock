# 기존 스마트원 함수를 이용해 안전한 결과만 반환한다.
param([Parameter(Mandatory=$true)][string]$ScriptPath,[Parameter(Mandatory=$true)][ValidateSet('CheckIn','CheckOut')][string]$Action)
$ErrorActionPreference = 'Stop'
$dispatched = $false
$mutex = $null
$locked = $false
try {
    . $ScriptPath -Action $Action -NoPause
    $dir = Split-Path -Parent $ScriptPath
    $config = Read-SmartOneConfig -Path (Join-Path $dir 'config.json')
    $sourcePath = Join-Path $dir 'smartone-page.js'
    $dataPath = Join-Path $env:LOCALAPPDATA 'SmartOne-Attendance'
    [void][IO.Directory]::CreateDirectory($dataPath)
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $mutex = New-Object Threading.Mutex($false, ('Local\SmartOne-Attendance-' + $sid))
    try { $locked = $mutex.WaitOne(0) } catch [Threading.AbandonedMutexException] { $locked = $true }
    if (-not $locked) { throw 'busy' }
    $page = Open-SmartOnePage -ProfilePath (Join-Path $dataPath 'chrome-profile') -ChromePath (Find-Chrome) -Url $config.url
    $settings = @{url=$config.url;username=$config.username;password=$config.password;action=$Action;dryRun=$false} | ConvertTo-Json -Compress
    $source = Get-Content -LiteralPath $sourcePath -Raw -Encoding UTF8
    $expression = $source + [Environment]::NewLine + "smartoneAttendance($settings)"
    $dispatched = $true
    $result = Invoke-CdpValue -WebSocketUrl $page.WebSocketUrl -Port $page.Port -Expression $expression -TimeoutSec 300
    if (-not $result -or $result.ok -isnot [bool] -or $result.code -isnot [string]) { throw 'invalid result' }
    $safeCode = $result.code -replace '[^A-Za-z0-9_-]', ''
    $line = '{0} {1} {2}' -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'), $Action, $safeCode
    try { Add-Content -LiteralPath (Join-Path $dataPath 'log.txt') -Value $line -Encoding UTF8 } catch { }
    @{code=$safeCode;date=([string]$result.date -replace '[^0-9-]','');time=([string]$result.time -replace '[^0-9:]','')} | ConvertTo-Json -Compress
} catch {
    @{code=$(if ($dispatched) {'uncertain'} else {'failed'});date='';time=''} | ConvertTo-Json -Compress
} finally {
    if ($config) { $config.password = $null }
    $settings = $null; $expression = $null
    if ($locked) { $mutex.ReleaseMutex() }
    if ($mutex) { $mutex.Dispose() }
}
