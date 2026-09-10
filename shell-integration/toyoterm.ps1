# toyoterm shell integration for PowerShell with PSReadLine.
if ($Host.Name -ne 'ConsoleHost' -or $env:TERM_PROGRAM -ne 'toyoterm' -or
    $global:TOYOTERM_SHELL_INTEGRATION_LOADED) { return }
$global:TOYOTERM_SHELL_INTEGRATION_LOADED = $true
[Console]::Write("`e]1337;ShellIntegrationVersion=1;powershell`e\")

function global:__ToyotermWriteCwd {
    $rawPath = (Get-Location).Path.Replace('\', '/')
    if (-not $rawPath.StartsWith('/')) { $rawPath = "/$rawPath" }
    $path = [Uri]::EscapeDataString($rawPath).Replace('%2F', '/')
    [Console]::Write("`e]7;file://$path`e\")
}

function global:__ToyotermWriteRemoteHost {
    $hostName = [System.Net.Dns]::GetHostName()
    if ($hostName) {
        [Console]::Write("`e]1337;RemoteHost=$([Environment]::UserName)@$hostName`e\")
    }
}

$global:__ToyotermPreviousPrompt = $function:prompt
function global:prompt {
    $succeeded = $?
    $nativeStatus = $global:LASTEXITCODE
    $status = if ($null -ne $nativeStatus) { $nativeStatus } elseif ($succeeded) { 0 } else { 1 }
    [Console]::Write("`e]133;D;$status`e\")
    __ToyotermWriteCwd
    __ToyotermWriteRemoteHost
    [Console]::Write("`e]133;A`e\")
    $promptText = if ($global:__ToyotermPreviousPrompt) {
        & $global:__ToyotermPreviousPrompt
    } else {
        "PS $PWD> "
    }
    "$promptText`e]133;B`e\"
}

if (Get-Module -ListAvailable PSReadLine) {
    $global:__ToyotermPreviousHistoryHandler = (Get-PSReadLineOption).AddToHistoryHandler
    Set-PSReadLineOption -AddToHistoryHandler {
        param($line)
        [Console]::Write("`e]133;C`e\")
        if ($global:__ToyotermPreviousHistoryHandler) {
            return & $global:__ToyotermPreviousHistoryHandler $line
        }
        $true
    }
}
