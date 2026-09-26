[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath,
    [string]$StartMenuDirectory = ""
)

$ErrorActionPreference = "Stop"
$installer = (Resolve-Path -LiteralPath $InstallerPath).Path
$testRoot = Join-Path $env:RUNNER_TEMP ("toyoterm-msi-test-" + [guid]::NewGuid().ToString("N"))
$destination = Join-Path $testRoot "installed"
$installLog = Join-Path $testRoot "install.log"
$uninstallLog = Join-Path $testRoot "uninstall.log"
New-Item -ItemType Directory -Force -Path $testRoot | Out-Null

function Invoke-MsiExec {
    param([string[]]$Arguments, [string]$LogPath)
    $process = Start-Process msiexec.exe -Wait -PassThru -ArgumentList ($Arguments + @("/qn", "/norestart", "/l*v", "`"$LogPath`""))
    if ($process.ExitCode -ne 0) {
        throw "msiexec exited with $($process.ExitCode); see $LogPath"
    }
}

$installArguments = @("/i", "`"$installer`"", "INSTALLFOLDER=`"$destination`"")
if ($StartMenuDirectory) {
    $installArguments += "APPLICATIONPROGRAMSFOLDER=`"$StartMenuDirectory`""
}
Invoke-MsiExec -Arguments $installArguments -LogPath $installLog
foreach ($file in @("toyoterm.exe", "toyoterm-gui.exe", "conpty.dll", "OpenConsole.exe")) {
    if (-not (Test-Path -LiteralPath (Join-Path $destination $file) -PathType Leaf)) {
        throw "MSI did not install $file"
    }
}
$reportedVersion = & (Join-Path $destination "toyoterm.exe") version
if ($reportedVersion -notmatch '^toyoterm [0-9]+\.[0-9]+\.[0-9]+$') {
    throw "MSI installed executable reported $reportedVersion"
}
$shortcutDirectory = if ($StartMenuDirectory) { $StartMenuDirectory } else { Join-Path ([Environment]::GetFolderPath("Programs")) "toyoterm" }
$shortcut = Join-Path $shortcutDirectory "toyoterm.lnk"
if (-not (Test-Path -LiteralPath $shortcut -PathType Leaf)) {
    throw "MSI did not create the Start Menu shortcut"
}
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (-not (($userPath -split ';' | ForEach-Object { $_.TrimEnd('\') }) -contains $destination.TrimEnd('\'))) {
    throw "MSI did not add the install directory to the user PATH"
}

$uninstallArguments = @("/x", "`"$installer`"", "INSTALLFOLDER=`"$destination`"")
if ($StartMenuDirectory) {
    $uninstallArguments += "APPLICATIONPROGRAMSFOLDER=`"$StartMenuDirectory`""
}
Invoke-MsiExec -Arguments $uninstallArguments -LogPath $uninstallLog
foreach ($file in @("toyoterm.exe", "toyoterm-gui.exe", "conpty.dll", "OpenConsole.exe")) {
    if (Test-Path -LiteralPath (Join-Path $destination $file)) {
        throw "MSI uninstall left $file behind"
    }
}
if (Test-Path -LiteralPath $shortcut) {
    throw "MSI uninstall left the Start Menu shortcut behind"
}
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (($userPath -split ';' | ForEach-Object { $_.TrimEnd('\') }) -contains $destination.TrimEnd('\')) {
    throw "MSI uninstall left the user PATH entry behind"
}
Write-Host "Windows MSI install and uninstall passed: $reportedVersion"
