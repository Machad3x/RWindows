param(
  [int]$Seconds = 15,
  [string]$Binary = 'target\debug\examples\taskbar.exe',
  [string]$Shot = 'C:\Users\machadex\AppData\Local\Temp\opencode\shell_real_test.png'
)
# Tests the real shell path without changing the Shell registry value.
# Explorer is stopped only for the duration and always restored in finally.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root $Binary
try {
  Get-Process explorer -ErrorAction SilentlyContinue | Stop-Process -Force
  # Windows auto-restarts Explorer, so keep it down for the test window.
  $keeper = Start-Job -ScriptBlock {
    param($ticks) for ($i = 0; $i -lt $ticks; $i++) {
      Get-Process explorer -ErrorAction SilentlyContinue | Stop-Process -Force
      Start-Sleep -Milliseconds 400
    }
  } -ArgumentList ($Seconds * 3)
  Start-Sleep -Seconds 1
  $shell = Start-Process -FilePath $exe -ArgumentList '--shell' -PassThru `
    -RedirectStandardError 'C:\Users\machadex\AppData\Local\Temp\opencode\shell_real_err.txt'
  Write-Output "shell pid=$($shell.Id); waiting $Seconds s"
  Start-Sleep -Seconds ([Math]::Min($Seconds, 8))
  if (Test-Path 'C:\Users\machadex\AppData\Local\Temp\opencode\shot.ps1') {
    & 'C:\Users\machadex\AppData\Local\Temp\opencode\shot.ps1' -Out $Shot
  }
  if (-not $shell.HasExited) { $shell.WaitForExit(($Seconds - 8) * 1000) | Out-Null }
}
finally {
  if ($keeper) { Stop-Job $keeper -ErrorAction SilentlyContinue; Remove-Job $keeper -Force -ErrorAction SilentlyContinue }
  Get-Process taskbar -ErrorAction SilentlyContinue | Stop-Process -Force
  if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) {
    Start-Process explorer.exe
  }
  Write-Output 'Explorer restored; registry untouched.'
}
