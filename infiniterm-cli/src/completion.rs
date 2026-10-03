//! `ift completion zsh`: the zsh completion function, printed so `ift
//! install` can write it into an fpath directory and a shell can eval it.
//! Kept as text in the binary rather than a file in the bundle so the
//! completion and the command list ship together and cannot drift; the test
//! below holds them to each other.
//!
//! What it completes: the subcommands with one-line descriptions, plus a
//! path (the bare `ift <path>` form); for `attach`, the live sessions and
//! the card numbers from `ift sessions`, which prints tab-separated rows
//! into a pipe for exactly this; for `group`, the groups `ift ls` shows;
//! directories for `diff`, `install-pi-hooks` and `install-extension`.
//! Related: main.rs (USAGE,
//! the dispatch), attach.rs (the columns the pipe form prints).

/// The `_ift` function. `#compdef` first, so it works from an fpath dir.
pub const ZSH: &str = r#"#compdef ift
# zsh completion for ift, from `ift completion zsh`. Do not edit: the next
# `ift install` rewrites it.

_ift_sessions() {
  # `ift sessions` into a pipe: id, pid, cwd, cmd, started, #card, label.
  local -a rows
  local id pid cwd cmd started card label
  while IFS=$'\t' read -r id pid cwd cmd started card label; do
    [[ -z $id ]] && continue
    label=${label//:/\\:}
    if [[ $card == \#* ]]; then
      rows+=("${card#\#}:${label} (${id})")
    fi
    rows+=("${id}:${label:-$cmd}")
  done < <(ift sessions 2>/dev/null)
  _describe -t sessions 'session or card number' rows
}

_ift_cards() {
  # `ift ls` into a pipe: id, group, directory, state, remote, #card.
  local -a rows
  local id group dir state remote card
  while IFS=$'\t' read -r id group dir state remote card; do
    [[ $card == \#* ]] && rows+=("${card#\#}:${dir//:/\\:} (${state})")
  done < <(ift ls 2>/dev/null)
  _describe -t cards 'card number' rows
}

_ift_command_ids() {
  # `ift commands` into a pipe: id, label, key.
  local -a rows
  local id label key
  while IFS=$'\t' read -r id label key; do
    [[ -n $id ]] && rows+=("${id}:${label//:/\\:}")
  done < <(ift commands 2>/dev/null)
  _describe -t commands 'command id' rows
}

_ift_groups() {
  # `ift ls` into a pipe: id, group, directory, state, remote, card.
  local -a groups
  local id group rest
  while IFS=$'\t' read -r id group rest; do
    [[ -n $group && $group != - ]] && groups+=("${group//:/\\:}")
  done < <(ift ls 2>/dev/null)
  _describe -t groups 'group' groups
}

_ift() {
  local -a cmds
  cmds=(
    'diff:changes against git HEAD, as a card'
    'ls:cards as a table'
    'sessions:session daemons still running, app or no app'
    'attach:connect a session'\''s shell to this terminal'
    'connect:open a second infiniterm for a host'
    'send:type into a card'\''s shell'
    'read:print what a card shows'
    'close:close a card'
    'run:run a registered command by id'
    'omni:what the omnibox would show for a term'
    'commands:every command the app registers, with its key'
    'usage:the commands and gestures you used, and the ones you never did'
    'licence:register this Mac with its commercial licence key'
    'name:name the card this is run from'
    'group:put this card in a group'
    'install:put ift on $PATH and this completion on fpath'
    'install-claude-hooks:wire infiniterm into ~/.claude/settings.json'
    'install-codex-hooks:wire infiniterm into ~/.codex/hooks.json'
    'install-opencode-hooks:install the OpenCode plugin'
    'install-pi-hooks:install the Pi extension'
    'install-extension:add an extension the browser cards load'
    'completion:print the zsh completion'
  )
  if (( CURRENT == 2 )); then
    _describe -t commands 'command' cmds
    _files
    return
  fi
  case $words[2] in
    attach) (( CURRENT == 3 )) && _ift_sessions ;;
    connect) _arguments '--name[a name for its window]:name:' '--color[its colour, six hex digits]:colour:' '--check[only test the host]' '1:host:_hosts' ;;
    send) (( CURRENT == 3 )) && _ift_cards ;;
    read) (( CURRENT == 3 )) && _ift_cards || _arguments '--lines[the last N lines only]:lines:' '--all[include the history]' ;;
    close) (( CURRENT == 3 )) && _ift_cards ;;
    run) (( CURRENT == 3 )) && _ift_command_ids ;;
    diff) _files -/ ;;
    group) _ift_groups ;;
    install-pi-hooks) _arguments '--dry-run[show the edit, write nothing]' '1:agent dir:_files -/' ;;
    install-claude-hooks) _arguments '--dry-run[show the edit, write nothing]' ;;
    install-codex-hooks) _arguments '--dry-run[show the edit, write nothing]' ;;
    install-opencode-hooks) _arguments '--dry-run[show the edit, write nothing]' ;;
    install-extension) _files -/ ;;
    sessions) _arguments '--full[every column: id, pid, cwd, command, started]' ;;
    completion) _values 'shell' zsh ;;
    omni) _message 'a term for the omnibox' ;;
    name) _message 'a name for this card' ;;
  esac
}

_ift "$@"
"#;

/// Where `ift install` writes `_ift`: a per-user fpath directory, made if
/// missing. Not Homebrew's site-functions, which is Homebrew's to own.
pub fn install_dir(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".local").join("share").join("zsh").join("site-functions")
}

/// Writes the function, and says what the shell profile needs when the
/// directory is not on `$FPATH` yet: `fpath` is per shell, so this cannot
/// be done for the user the way the symlink can.
pub fn install(home: &std::path::Path, fpath: Option<&str>) -> Result<Vec<String>, String> {
    let dir = install_dir(home);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let file = dir.join("_ift");
    std::fs::write(&file, ZSH).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
    let mut lines = vec![format!("ift: zsh completion at {}", file.display())];
    let on_fpath = fpath
        .map(|f| f.split(':').any(|d| std::path::Path::new(d) == dir))
        .unwrap_or(false);
    if !on_fpath {
        lines.push(format!(
            "ift: add this before compinit in ~/.zshrc, then open a new shell:\n  fpath=({} $fpath)",
            dir.display()
        ));
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every subcommand main.rs dispatches is in the completion, and nothing
    /// the completion offers is unknown to main.rs.
    #[test]
    fn the_completion_lists_exactly_the_subcommands() {
        let listed: Vec<&str> = ZSH
            .lines()
            .filter_map(|l| l.trim().strip_prefix('\''))
            .filter_map(|l| l.split_once(':'))
            .map(|(name, _)| name)
            .collect();
        let mut expected = crate::SUBCOMMANDS.to_vec();
        expected.sort();
        let mut got = listed.clone();
        got.sort();
        assert_eq!(got, expected);
    }

    #[test]
    fn install_writes_the_function_and_names_the_fpath_line_only_when_needed() {
        let tmp = std::env::temp_dir().join(format!("ift-completion-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let dir = install_dir(&tmp);
        let lines = install(&tmp, Some("/usr/share/zsh/site-functions")).unwrap();
        assert!(std::fs::read_to_string(dir.join("_ift")).unwrap().starts_with("#compdef ift"));
        assert_eq!(lines.len(), 2, "not on fpath: says what to add");
        let on = format!("/x:{}:/y", dir.display());
        let lines = install(&tmp, Some(&on)).unwrap();
        assert_eq!(lines.len(), 1, "on fpath: nothing to add");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
