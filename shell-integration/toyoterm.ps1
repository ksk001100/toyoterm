# toyoterm shell integration for PowerShell with PSReadLine.
if ($Host.Name -ne 'ConsoleHost' -or $env:TERM_PROGRAM -ne 'toyoterm' -or
    $global:TOYOTERM_SHELL_INTEGRATION_LOADED) { return }
$global:TOYOTERM_SHELL_INTEGRATION_LOADED = $true
[Console]::Write("$([char]27)]1337;ShellIntegrationVersion=1;powershell$([char]27)\")

function global:__ToyotermWriteCwd {
    $location = Get-Location
    if ($location.Provider.Name -ne 'FileSystem') { return }
    $rawPath = $location.ProviderPath.Replace('\', '/')
    if (-not $rawPath.StartsWith('/')) { $rawPath = "/$rawPath" }
    $path = [Uri]::EscapeDataString($rawPath).Replace('%2F', '/')
    [Console]::Write("$([char]27)]7;file://$path$([char]27)\")
}

function global:__ToyotermWriteRemoteHost {
    $hostName = [System.Net.Dns]::GetHostName()
    if ($hostName) {
        [Console]::Write("$([char]27)]1337;RemoteHost=$([Environment]::UserName)@$hostName$([char]27)\")
    }
}

__ToyotermWriteCwd
__ToyotermWriteRemoteHost

$global:__ToyotermInstalledPrompt = {
    $succeeded = $?
    $nativeStatus = $global:LASTEXITCODE
    $status = if ($succeeded) { 0 } elseif ($null -ne $nativeStatus -and $nativeStatus -ne 0) { $nativeStatus } else { 1 }
    [Console]::Write("$([char]27)]133;D;$status$([char]27)\")
    __ToyotermWriteCwd
    __ToyotermWriteRemoteHost
    [Console]::Write("$([char]27)]133;A$([char]27)\")
    $promptText = if ($global:__ToyotermPreviousPrompt) {
        (& $global:__ToyotermPreviousPrompt) -join "`n"
    } else {
        "PS $PWD> "
    }
    "$promptText$([char]27)]133;B$([char]27)\"
}

$global:__ToyotermPreviousPrompt = $function:prompt
$function:global:prompt = $global:__ToyotermInstalledPrompt

if (Get-Module -ListAvailable PSReadLine) {
    $global:__ToyotermPreviousHistoryHandler = (Get-PSReadLineOption).AddToHistoryHandler
    Set-PSReadLineOption -AddToHistoryHandler {
        param($line)
        [Console]::Write("$([char]27)]133;C$([char]27)\")
        if ($function:prompt -ne $global:__ToyotermInstalledPrompt) {
            $global:__ToyotermPreviousPrompt = $function:prompt
            $function:global:prompt = $global:__ToyotermInstalledPrompt
        }
        if ($global:__ToyotermPreviousHistoryHandler) {
            return & $global:__ToyotermPreviousHistoryHandler $line
        }
        $true
    }
}
