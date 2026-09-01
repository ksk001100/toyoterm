# toyoterm shell integration for fish. Pipe this file to `source` in interactive fish.
status is-interactive; and test "$TERM_PROGRAM" = toyoterm; or return 0
set -q TOYOTERM_SHELL_INTEGRATION_LOADED; and return 0
set -gx TOYOTERM_SHELL_INTEGRATION_LOADED 1

function __toyoterm_urlencode_path --argument-names value
    string escape --style=url -- $value | string replace -a '%2F' '/'
end

function __toyoterm_command_start --on-event fish_preexec
    printf '\e]133;C\e\\'
end

function __toyoterm_command_end --on-event fish_postexec
    set -l command_status $status
    printf '\e]133;D;%d\e\\' $command_status
end

function __toyoterm_cwd --on-event fish_prompt
    printf '\e]7;file://%s\e\\' (__toyoterm_urlencode_path $PWD)
end
