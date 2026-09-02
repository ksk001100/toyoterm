[CmdletBinding()]
param(
    [string]$InstallDirectory = (Join-Path $env:LOCALAPPDATA "Programs\toyoterm"),
    [switch]$NoPath,
    [switch]$NoStartMenu
)

$ErrorActionPreference = "Stop"
$sourceDirectory = $PSScriptRoot
$sourceExecutable = Join-Path $sourceDirectory "toyoterm.exe"
if (-not (Test-Path -LiteralPath $sourceExecutable -PathType Leaf)) {
    throw "toyoterm.exe was not found next to this installer"
}

$resolvedInstallDirectory = [System.IO.Path]::GetFullPath($InstallDirectory)
New-Item -ItemType Directory -Force -Path $resolvedInstallDirectory | Out-Null
Copy-Item -Force -LiteralPath $sourceExecutable -Destination (Join-Path $resolvedInstallDirectory "toyoterm.exe")
Copy-Item -Force -LiteralPath (Join-Path $sourceDirectory "Uninstall-Toyoterm.ps1") -Destination $resolvedInstallDirectory

if (-not $NoPath) {
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $entries = @($userPath -split ";" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if (-not ($entries | Where-Object { $_.TrimEnd("\") -ieq $resolvedInstallDirectory.TrimEnd("\") })) {
        $entries += $resolvedInstallDirectory
        [Environment]::SetEnvironmentVariable("Path", ($entries -join ";"), "User")
    }
}

if (-not $NoStartMenu) {
    $startMenuDirectory = Join-Path ([Environment]::GetFolderPath("Programs")) "toyoterm"
    New-Item -ItemType Directory -Force -Path $startMenuDirectory | Out-Null
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut((Join-Path $startMenuDirectory "toyoterm.lnk"))
    $installedExecutable = Join-Path $resolvedInstallDirectory "toyoterm.exe"
    $shortcut.TargetPath = $installedExecutable
    $shortcut.IconLocation = "$installedExecutable,0"
    $shortcut.WorkingDirectory = $env:USERPROFILE
    $shortcut.Description = "Programmable terminal emulator"
    $shortcut.Save()
}

Write-Host "Installed toyoterm to $resolvedInstallDirectory"
if (-not $NoPath) {
    Write-Host "Open a new terminal before using toyoterm from PATH."
}
Write-Host "Uninstall with: powershell -ExecutionPolicy Bypass -File `"$(Join-Path $resolvedInstallDirectory 'Uninstall-Toyoterm.ps1')`""
