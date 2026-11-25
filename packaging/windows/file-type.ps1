# Opens .sparksmith files with the Sparksmith in this folder, for this user
# only; or no longer.
#   powershell -ExecutionPolicy Bypass -File file-type.ps1
#   powershell -ExecutionPolicy Bypass -File file-type.ps1 -Remove
param([switch]$Remove)
$ErrorActionPreference = 'Stop'
$classes = 'HKCU:\Software\Classes'
$type = "$classes\io.sparksmith.project"
$extension = "$classes\.sparksmith"
if ($Remove) {
    if (Test-Path $type) { Remove-Item $type -Recurse }
    if ((Test-Path $extension) -and
        (Get-Item $extension).GetValue('') -eq 'io.sparksmith.project') {
        Remove-Item $extension -Recurse
    }
    Write-Output '.sparksmith files are no longer opened by Sparksmith'
    exit
}
$app = Join-Path $PSScriptRoot 'sparksmith-desktop.exe'
if (-not (Test-Path $app)) { throw "$app is not there" }
New-Item "$type\shell\open\command" -Force | Out-Null
Set-Item $type 'Sparksmith project'
Set-Item "$type\shell\open\command" "`"$app`" `"%1`""
New-Item $extension -Force | Out-Null
Set-Item $extension 'io.sparksmith.project'
Write-Output ".sparksmith files open with $app"
