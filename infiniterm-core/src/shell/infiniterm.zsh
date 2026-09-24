# infiniterm's command marks (OSC 133), sourced by the .zshenv beside it for
# an interactive shell. preexec says a command started (133;C); the next
# precmd says it finished and with what status (133;D;<status>). A bare
# Enter runs no command, so it reports nothing. Our precmd goes FIRST in the
# list, because $? is the command's status only until another precmd
# function has run.
builtin typeset -gi _ift_running=0
_ift_precmd() {
  builtin local ret=$?
  if (( _ift_running )); then
    builtin printf '\e]133;D;%d\a' $ret
    _ift_running=0
  fi
}
_ift_preexec() {
  _ift_running=1
  builtin printf '\e]133;C\a'
}
precmd_functions=(_ift_precmd ${precmd_functions[@]})
preexec_functions+=(_ift_preexec)
