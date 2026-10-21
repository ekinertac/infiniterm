# infiniterm's zsh shell integration, loaded through ZDOTDIR.
#
# The app starts a card's zsh with ZDOTDIR pointing here, so this file runs
# first. It puts ZDOTDIR back the way the user had it, reads the user's own
# .zshenv, and for an interactive shell loads infiniterm.zsh beside it,
# which marks each command with OSC 133 so the card can show a long command
# working, done or failed (program_state.rs). zsh then reads .zprofile,
# .zshrc and .zlogin from the restored ZDOTDIR as usual, so nobody's dotfiles
# change. Ghostty and VS Code load their integration the same way.
#
# Written at every launch by shell_integration.rs; edits here are lost.
if [[ -n "${INFINITERM_ZDOTDIR+X}" ]]; then
  builtin export ZDOTDIR="$INFINITERM_ZDOTDIR"
  builtin unset INFINITERM_ZDOTDIR
else
  builtin unset ZDOTDIR
fi
{
  builtin typeset _ift_f="${ZDOTDIR-$HOME}/.zshenv"
  [[ ! -r "$_ift_f" ]] || builtin source -- "$_ift_f"
} always {
  if [[ -o interactive ]]; then
    builtin typeset _ift_f="${${(%):-%x}:A:h}/infiniterm.zsh"
    [[ ! -r "$_ift_f" ]] || builtin source -- "$_ift_f"
  fi
  builtin unset _ift_f
}
