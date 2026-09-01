# toyoterm shell integration for bash. Source this file from interactive bash only.
[[ $- == *i* && ${TERM_PROGRAM-} == toyoterm && -z ${TOYOTERM_SHELL_INTEGRATION_LOADED-} ]] || return 0
TOYOTERM_SHELL_INTEGRATION_LOADED=1

__toyoterm_urlencode_path() {
  local LC_ALL=C value=$1 result= char hex i
  for ((i = 0; i < ${#value}; i++)); do
    char=${value:i:1}
    case $char in
      [a-zA-Z0-9/_.~-]) result+=$char ;;
      *) printf -v hex '%%%02X' "'$char"; result+=$hex ;;
    esac
  done
  printf '%s' "$result"
}

__toyoterm_prompt() {
  local status=$?
  printf '\e]133;D;%d\e\\' "$status"
  printf '\e]7;file://%s\e\\' "$(__toyoterm_urlencode_path "$PWD")"
  return "$status"
}

__toyoterm_command_start() {
  printf '\e]133;C\e\\'
}

# PS0 is expanded after a complete command is read and immediately before it runs.
PS0='$(__toyoterm_command_start)'${PS0-}
if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == 'declare -a'* ]]; then
  PROMPT_COMMAND=(__toyoterm_prompt "${PROMPT_COMMAND[@]}")
else
  PROMPT_COMMAND="__toyoterm_prompt${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
fi
