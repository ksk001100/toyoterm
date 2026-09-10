# toyoterm shell integration for zsh. Source this file from interactive zsh only.
[[ -o interactive && ${TERM_PROGRAM-} == toyoterm && -z ${TOYOTERM_SHELL_INTEGRATION_LOADED-} ]] || return 0
typeset -g TOYOTERM_SHELL_INTEGRATION_LOADED=1
printf '\e]1337;ShellIntegrationVersion=1;zsh\e\\'
autoload -Uz add-zsh-hook

__toyoterm_urlencode_path() {
  local value=$1 result='' char hex i
  local LC_ALL=C
  for ((i = 1; i <= ${#value}; i++)); do
    char=${value[i]}
    case $char in
      [a-zA-Z0-9/_.~-]) result+=$char ;;
      *) printf -v hex '%%%02X' "'$char"; result+=$hex ;;
    esac
  done
  print -rn -- "$result"
}

__toyoterm_preexec() {
  printf '\e]133;C\e\\'
}

__toyoterm_precmd() {
  local status=$?
  printf '\e]133;D;%d\e\\' "$status"
  printf '\e]7;file://%s\e\\' "$(__toyoterm_urlencode_path "$PWD")"
  [[ -n ${HOST-} ]] && printf '\e]1337;RemoteHost=%s@%s\e\\' "${USERNAME-}" "$HOST"
  printf '\e]133;A\e\\'
}

add-zsh-hook preexec __toyoterm_preexec
add-zsh-hook precmd __toyoterm_precmd
PROMPT+=$'%{\e]133;B\e\\%}'
