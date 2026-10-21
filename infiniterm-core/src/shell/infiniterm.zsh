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

# Selecting in the command line the way a Mac input field does (2026-09-27):
# Shift+Left/Right a character, Shift+Alt+Left/Right a word, Shift+Home/End
# to the line's ends; typing, Backspace, Delete or a paste replace the
# selection, any other key drops it and does its own job. zsh has the
# region (the mark and the cursor) but no key selects it, so these widgets
# set the mark on the first Shift move and switch to the `ift-select`
# keymap until the selection ends, the zsh-shift-select plugin's way.
# Every change is reported as an OSC 52 "selection" (not the clipboard), so
# Cmd+C and Cmd+X in the card can copy what zsh has selected.
_ift_region() {
  builtin local text=
  if (( REGION_ACTIVE )); then
    if (( MARK < CURSOR )); then
      text=${BUFFER[MARK+1,CURSOR]}
    else
      text=${BUFFER[CURSOR+1,MARK]}
    fi
  fi
  builtin printf '\e]52;s;%s\a' "$(builtin print -rn -- "$text" | command base64)" >/dev/tty
}
_ift_select() {
  if (( ! REGION_ACTIVE )); then
    zle set-mark-command -w
    zle -K ift-select
  fi
  zle ${WIDGET#_ift_select_} -w
  _ift_region
}
_ift_unselect() {
  zle deactivate-region -w
  zle -K main
  _ift_region
}
# Any other key: the selection goes and the key does what it always does.
_ift_deselect_and_input() {
  _ift_unselect
  zle -U -- "$KEYS"
}
# A character typed, or a paste starting: it replaces the selection.
_ift_replace_and_input() {
  zle kill-region -w
  _ift_unselect
  zle -U -- "$KEYS"
}
_ift_delete_selection() {
  zle kill-region -w
  _ift_unselect
}
typeset -a _ift_moves=(backward-char forward-char backward-word forward-word beginning-of-line end-of-line)
for _ift_w in $_ift_moves; do
  zle -N _ift_select_$_ift_w _ift_select
done
unset _ift_w
zle -N _ift_deselect_and_input
zle -N _ift_replace_and_input
zle -N _ift_delete_selection
# Bound at the first prompt, after the user's .zshrc, so a `bindkey -v` or a
# framework that resets the keymaps there does not drop them. The user's own
# bindings for these keys are overridden: the sequences are what a Shift
# arrow sends, and nothing binds them by default.
_ift_bind_select() {
  precmd_functions=(${precmd_functions:#_ift_bind_select})
  bindkey -N ift-select
  bindkey -M ift-select -R '^@'-'^?' _ift_deselect_and_input
  bindkey -M ift-select -R ' '-'~' _ift_replace_and_input
  bindkey -M ift-select -R '\M-^@'-'\M-^?' _ift_replace_and_input
  bindkey -M ift-select '^?' _ift_delete_selection
  bindkey -M ift-select '^H' _ift_delete_selection
  bindkey -M ift-select '^[[3~' _ift_delete_selection
  bindkey -M ift-select '^[[200~' _ift_replace_and_input
  builtin local seq w
  for seq w in '^[[1;2D' backward-char '^[[1;2C' forward-char \
    '^[[1;4D' backward-word '^[[1;4C' forward-word \
    '^[[1;2H' beginning-of-line '^[[1;2F' end-of-line; do
    bindkey "$seq" _ift_select_$w
    bindkey -M ift-select "$seq" _ift_select_$w
  done
  # Cmd+Backspace sends Ctrl+U (readline's delete to the line start, which
  # Claude Code and bash do); zsh's default for it is the whole line.
  bindkey '^U' backward-kill-line
}
precmd_functions+=(_ift_bind_select)
