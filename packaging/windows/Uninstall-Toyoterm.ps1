[CmdletBinding()]
param(
    [string]$InstallDirectory = $PSScriptRoot,
    [switch]$KeepPath,
    [switch]$KeepStartMenu
)

$ErrorActionPreference = "Stop"
$resolvedInstallDirectory = [System.IO.Path]::GetFullPath($InstallDirectory)

if (-not $KeepPath) {
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $entries = @($userPath -split ";" | Where-Object {
        -not [string]::IsNullOrWhiteSpace($_) -and
        $_.TrimEnd("\") -ine $resolvedInstallDirectory.TrimEnd("\")
    })
    [Environment]::SetEnvironmentVariable("Path", ($entries -join ";"), "User")
}

if (-not $KeepStartMenu) {
    $startMenuDirectory = Join-Path ([Environment]::GetFolderPath("Programs")) "toyoterm"
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue -LiteralPath $startMenuDirectory
}

$executable = Join-Path $resolvedInstallDirectory "toyoterm.exe"
Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath $executable

$installedUninstaller = Join-Path $resolvedInstallDirectory "Uninstall-Toyoterm.ps1"
Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath $installedUninstaller
Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath $resolvedInstallDirectory

Write-Host "Uninstalled toyoterm from $resolvedInstallDirectory"
