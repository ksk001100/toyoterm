[CmdletBinding()]
param(
    [string]$InstallDirectory = "",
    [switch]$KeepPath,
    [switch]$KeepStartMenu
)

$ErrorActionPreference = "Stop"
$scriptPath = $MyInvocation.MyCommand.Path
if ([string]::IsNullOrWhiteSpace($InstallDirectory)) {
    $InstallDirectory = [System.IO.Path]::GetDirectoryName($scriptPath)
}
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

foreach ($installedFile in @("toyoterm.exe", "toyoterm-gui.exe", "conpty.dll", "OpenConsole.exe", "Uninstall-Toyoterm.ps1")) {
    $installedPath = Join-Path $resolvedInstallDirectory $installedFile
    for ($attempt = 0; $attempt -lt 10; $attempt++) {
        Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath $installedPath
        if (-not (Test-Path -LiteralPath $installedPath)) {
            break
        }
        Start-Sleep -Milliseconds 100
    }
}
try {
    [System.IO.Directory]::Delete($resolvedInstallDirectory, $false)
} catch [System.IO.IOException] {
    # Preserve an install directory that contains files not owned by toyoterm.
} catch [System.UnauthorizedAccessException] {
    # The installed files are already removed; a locked directory can remain.
}

Write-Host "Uninstalled toyoterm from $resolvedInstallDirectory"
