# toyoterm shell integration for PowerShell with PSReadLine.
if ($Host.Name -ne 'ConsoleHost' -or $env:TERM_PROGRAM -ne 'toyoterm' -or
    $global:TOYOTERM_SHELL_INTEGRATION_LOADED) { return }
$global:TOYOTERM_SHELL_INTEGRATION_LOADED = $true

function global:__ToyotermWriteCwd {
    $rawPath = (Get-Location).Path.Replace('\', '/')
    if (-not $rawPath.StartsWith('/')) { $rawPath = "/$rawPath" }
    $path = [Uri]::EscapeDataString($rawPath).Replace('%2F', '/')
    [Console]::Write("`e]7;file://$path`e\")
}

$global:__ToyotermPreviousPrompt = $function:prompt
function global:prompt {
    $succeeded = $?
    $nativeStatus = $global:LASTEXITCODE
    $status = if ($null -ne $nativeStatus) { $nativeStatus } elseif ($succeeded) { 0 } else { 1 }
    [Console]::Write("`e]133;D;$status`e\")
    __ToyotermWriteCwd
    if ($global:__ToyotermPreviousPrompt) { & $global:__ToyotermPreviousPrompt } else { "PS $PWD> " }
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
