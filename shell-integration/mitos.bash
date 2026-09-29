# MITOS Terminal — shell integration for bash.
#
# Emits OSC 7 (working directory) and OSC 133 (A prompt, B input, C output, D finished + exit code)
# so the terminal can offer: new tabs in the same directory, jump-to-previous-prompt, command
# history with exit codes/durations, and "command finished" notifications.
#
# mitos-terminal loads this automatically for bash. To enable it in other setups add to ~/.bashrc:
#   [ -n "$MITOS_SHELL_INTEGRATION_DIR" ] && . "$MITOS_SHELL_INTEGRATION_DIR/mitos.bash"

[ -n "$MITOS_SHELL_INTEGRATION_LOADED" ] && return 0
[ "$TERM_PROGRAM" = "mitos-terminal" ] || return 0
case $- in *i*) ;; *) return 0 ;; esac
MITOS_SHELL_INTEGRATION_LOADED=1

__mitos_cwd_uri() {
  local LC_ALL=C str="$PWD" i c out="" hex
  for (( i = 0; i < ${#str}; i++ )); do
    c="${str:i:1}"
    case "$c" in
      [a-zA-Z0-9/._~-]) out+="$c" ;;
      *) printf -v hex '%%%02X' "'$c"; out+="$hex" ;;
    esac
  done
  printf 'file://%s%s' "${HOSTNAME:-localhost}" "$out"
}

# Runs first in PROMPT_COMMAND: captures $? before anything else can change it.
__mitos_precmd_first() {
  __mitos_ret=$?
  __mitos_in_prompt=1
  if [ -n "$__mitos_cmd_running" ]; then
    printf '\e]133;D;%s\a' "$__mitos_ret"
  fi
  __mitos_cmd_running=
  return "$__mitos_ret"
}

# Runs last: reports cwd and marks the start of the prompt, then restores $?.
__mitos_precmd_last() {
  local ret=$?
  printf '\e]7;%s\a' "$(__mitos_cwd_uri)"
  printf '\e]133;A\a'
  __mitos_in_prompt=
  return "$ret"
}

__mitos_preexec() {
  [ -n "$__mitos_in_prompt" ] && return
  [ -n "$__mitos_cmd_running" ] && return
  [ -n "$COMP_LINE" ] && return
  case "$BASH_COMMAND" in __mitos_*) return ;; esac
  __mitos_cmd_running=1
  printf '\e]133;C\a'
}

if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
  PROMPT_COMMAND=(__mitos_precmd_first "${PROMPT_COMMAND[@]}" __mitos_precmd_last)
else
  PROMPT_COMMAND="__mitos_precmd_first${PROMPT_COMMAND:+;$PROMPT_COMMAND};__mitos_precmd_last"
fi

# B: the prompt ends here (zero-width, wrapped in \[ \] so readline's width maths stays right).
PS1="${PS1}\[\e]133;B\a\]"

# C needs the DEBUG trap; never clobber one the user already installed.
if [ -z "$(trap -p DEBUG)" ]; then
  trap '__mitos_preexec' DEBUG
fi
