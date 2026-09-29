# MITOS Terminal — shell integration for zsh (OSC 7 + OSC 133).
# Add to ~/.zshrc:
#   [[ -n "$MITOS_SHELL_INTEGRATION_DIR" ]] && source "$MITOS_SHELL_INTEGRATION_DIR/mitos.zsh"

[[ -n "$MITOS_SHELL_INTEGRATION_LOADED" ]] && return 0
[[ "$TERM_PROGRAM" == "mitos-terminal" ]] || return 0
[[ -o interactive ]] || return 0
MITOS_SHELL_INTEGRATION_LOADED=1

autoload -Uz add-zsh-hook

__mitos_cwd_uri() {
  emulate -L zsh
  local LC_ALL=C str=$PWD out= c i
  for (( i = 1; i <= ${#str}; i++ )); do
    c=${str[i]}
    case $c in
      [a-zA-Z0-9/._~-]) out+=$c ;;
      *) out+=$(printf '%%%02X' "'$c") ;;
    esac
  done
  printf 'file://%s%s' "${HOST:-localhost}" "$out"
}

__mitos_precmd() {
  local ret=$?
  if [[ -n $__mitos_running ]]; then
    printf '\e]133;D;%s\a' "$ret"
  fi
  __mitos_running=
  printf '\e]7;%s\a' "$(__mitos_cwd_uri)"
  printf '\e]133;A\a'
}

__mitos_preexec() {
  __mitos_running=1
  printf '\e]133;C\a'
}

add-zsh-hook precmd __mitos_precmd
add-zsh-hook preexec __mitos_preexec

# B: end of prompt, zero width.
PS1="${PS1}%{$(printf '\e]133;B\a')%}"
