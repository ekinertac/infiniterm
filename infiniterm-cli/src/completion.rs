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
//! For `join`, `connect` and `phone add --host`, a host: the ssh aliases of
//! `~/.ssh/config` and of the files it Includes, the known hosts, with an
//! optional `user@` (`_ift_hosts`; zsh's `_hosts` alone reads /etc/hosts and
//! showed none of them). `phone remove` takes the paired phones.
//! Related: main.rs (USAGE,
//! the dispatch), attach.rs (the columns the pipe form prints).

// `ift completion zsh` still PRINTS on Windows (the text is the same, and
// a Windows box can be feeding a zsh on another machine), but nothing
// there installs it: `ift install` has no fpath to write into.
#![cfg_attr(windows, allow(dead_code))]

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

_ift_workspaces() {
  # `ift workspaces` into a pipe: id, name, cards, active. The name is what
  # a person types; `new` makes one.
  local -a names
  local id name rest
  while IFS=$'\t' read -r id name rest; do
    [[ -n $name && $name != - ]] && names+=("${name//:/\\:}")
  done < <(ift workspaces 2>/dev/null)
  names+=(new)
  _describe -t workspaces 'workspace' names
}

_ift_ssh_hosts() {
  # The Host aliases of ~/.ssh/config and of the files it Includes (a
  # relative path is under ~/.ssh, as ssh reads it), one per line, no wildcard
  # patterns. zsh's own `_hosts` reads only /etc/hosts, which is why `ift join
  # <Tab>` showed none of them.
  setopt local_options extended_glob null_glob
  local -a todo seen out
  local f line key rest pat
  local depth=0
  todo=(~/.ssh/config)
  while (( $#todo && depth++ < 50 )); do
    f=${todo[1]}
    todo=(${todo[2,-1]})
    (( ${seen[(Ie)$f]} )) && continue
    seen+=($f)
    [[ -r $f ]] || continue
    for line in ${(f)"$(<$f)"}; do
      line=${line##[[:space:]]#}
      key=${${(s: :)line}[1]:l}
      rest=${line#* }
      case $key in
        host)
          for pat in ${=rest}; do
            [[ $pat == *[\*\?\!]* ]] || out+=($pat)
          done ;;
        include)
          for pat in ${=rest}; do
            pat=${pat/#\~/$HOME}
            [[ $pat == /* ]] || pat=$HOME/.ssh/$pat
            todo+=(${~pat}(N))
          done ;;
      esac
    done
  done
  print -rl -- ${(u)out}
}

_ift_known_hosts() {
  # The plain host names in ~/.ssh/known_hosts, one per line. No hashed
  # entries, no patterns, and no bare IP addresses: a server reached by name
  # is in the file under its name and under its address, and the addresses
  # would bury the names.
  setopt local_options extended_glob null_glob
  local line pat
  local -a out
  [[ -r ~/.ssh/known_hosts ]] || return 0
  for line in ${(f)"$(<~/.ssh/known_hosts)"}; do
    [[ -z $line || $line == \|* || $line == \#* || $line == \@* ]] && continue
    for pat in ${(s:,:)${line%% *}}; do
      pat=${pat#\[}
      pat=${pat%%\]*}
      [[ $pat == *[\*\?\!]* || $pat == [0-9.]## || $pat == *:* ]] && continue
      out+=($pat)
    done
  done
  print -rl -- ${(u)out}
}

_ift_hosts() {
  # A host for join and connect: an ssh alias or known host, with an optional
  # `user@`, then whatever zsh's `_hosts` has.
  local -a aliases known
  aliases=(${(f)"$(_ift_ssh_hosts)"})
  known=(${(f)"$(_ift_known_hosts)"})
  compset -P '*@'
  _alternative \
    "ssh-hosts:ssh host:compadd -a aliases" \
    "known-hosts:known host:compadd -a known" \
    "hosts:host:_hosts"
}

_ift_phones() {
  # `ift phone list` prints one paired phone per line.
  local -a names
  names=(${(f)"$(ift phone list 2>/dev/null)"})
  _describe -t phones 'paired phone' names
}

_ift() {
  local -a cmds
  cmds=(
    'diff:changes against git HEAD, as a card'
    'ls:cards as a table'
    'workspaces:workspaces as a table'
    'workspace:saved workspaces: saved, save, open'
    'sessions:session daemons still running, app or no app'
    'attach:connect a session'\''s shell to this terminal'
    'connect:open a second infiniterm for a host'
    'serve:hold this Mac'\''s canvas for viewers with the app closed'
    'join:a viewer window on the canvas running on a host'
    'terminal:a terminal card in each folder'
    'send:type into a card'\''s shell'
    'read:print what a card shows'
    'browse:open a web address or a local file in a browser card'
    'focus:go to a card'
    'close:close a card'
    'clear-state:clear a card'\''s state colour'
    'move:move a card to another workspace'
    'card:lock a card: card protect <card> on|off'
    'run:run a registered command by id'
    'omni:what the omnibox would show for a term'
    'commands:every command the app registers, with its key'
    'usage:the commands and gestures you used, and the ones you never did'
    'update:the app update state: ift update status'
    'push:give a phone push id to this Mac: add, remove, list'
    'phone:pair a phone with this Mac: add, list, remove'
    'licence:register this Mac with its commercial licence key'
    'license:the same as licence'
    'update-key:make the key that signs Windows releases'
    'update-sign:sign a release manifest'
    'name:name the card this is run from'
    'group:put this card in a group'
    'install:put ift on $PATH and this completion on fpath'
    'install-claude-hooks:wire infiniterm into ~/.claude/settings.json'
    'install-codex-hooks:wire infiniterm into ~/.codex/hooks.json'
    'install-opencode-hooks:install the OpenCode plugin'
    'install-pi-hooks:install the Pi extension'
    'install-cursor-hooks:wire infiniterm into ~/.cursor/hooks.json'
    'install-antigravity-hooks:wire infiniterm into ~/.gemini/config/hooks.json'
    'install-extension:add an extension the browser cards load'
    'completion:print the zsh completion'
  )
  if (( CURRENT == 2 )); then
    _describe -t commands 'command' cmds
    _files
    return
  fi
  case $words[2] in
    serve) _arguments '--detach[run in the background]' '--stop[end the running one]' ;;
    attach) (( CURRENT == 3 )) && _ift_sessions ;;
    join) _arguments '--name[a name for its window]:name:' '--color[its colour, six hex digits]:colour:' '--check[only test the host]' '1:host:_ift_hosts' ;;
    connect) _arguments '--name[a name for its window]:name:' '--color[its colour, six hex digits]:colour:' '--check[only test the host]' '1:host:_ift_hosts' ;;
    browse) _arguments '*:file or address:_files' ;;
    send) (( CURRENT == 3 )) && _ift_cards ;;
    read) (( CURRENT == 3 )) && _ift_cards || _arguments '--lines[the last N lines only]:lines:' '--all[include the history]' '--ansi[the screen with its colours]' ;;
    focus) (( CURRENT == 3 )) && _ift_cards ;;
    close|clear-state) (( CURRENT == 3 )) && _ift_cards ;;
    move)
      case $CURRENT in
        3) _ift_cards ;;
        4) _ift_workspaces ;;
      esac ;;
    card)
      case $CURRENT in
        3) _values 'card verb' protect ;;
        4) _ift_cards ;;
        5) _values 'state' on off ;;
      esac ;;
    run) (( CURRENT == 3 )) && _ift_command_ids ;;
    diff) _files -/ ;;
    terminal) _arguments '--workspace[open on this workspace, or make one]:workspace:_ift_workspaces' '--name[a name for the new workspace]:name:' '*:folder:_files -/' ;;
    workspace)
      case $CURRENT in
        3) _values 'workspace verb' saved save open ;;
        4) [[ $words[3] == save ]] && _ift_workspaces
           [[ $words[3] == open ]] && _values 'saved workspace' ${(f)"$(ift workspace saved 2>/dev/null)"} ;;
      esac ;;
    group) _ift_groups ;;
    install-pi-hooks) _arguments '--dry-run[show the edit, write nothing]' '1:agent dir:_files -/' ;;
    install-claude-hooks) _arguments '--dry-run[show the edit, write nothing]' ;;
    install-codex-hooks) _arguments '--dry-run[show the edit, write nothing]' ;;
    install-opencode-hooks) _arguments '--dry-run[show the edit, write nothing]' ;;
    install-cursor-hooks) _arguments '--dry-run[show the edit, write nothing]' ;;
    install-antigravity-hooks) _arguments '--dry-run[show the edit, write nothing]' ;;
    install-extension) _files -/ ;;
    sessions) _arguments '--full[every column: id, pid, cwd, command, started]' ;;
    completion) _values 'shell' zsh ;;
    update) _values 'word' status ;;
    push) _values 'word' add remove list ;;
    phone)
      if (( CURRENT == 3 )); then
        _values 'word' add list remove
      else
        case $words[3] in
          remove) (( CURRENT == 4 )) && _ift_phones ;;
          add) _arguments '--host[the address in the QR code]:host:_ift_hosts' '--port[the ssh port in the QR code]:port:' '1:name:' ;;
        esac
      fi ;;
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

    // unix only: `$FPATH` is a colon-separated zsh variable, and a Windows
    // path has a colon two characters in. The feature is zsh's anyway;
    // `ift install` on Windows writes no completion at all.
    #[cfg(unix)]
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

    fn zsh_here() -> bool {
        std::process::Command::new("zsh")
            .arg("--version")
            .output()
            .is_ok()
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ift-zsh-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// `zsh -n`: the whole completion parses. Skipped where there is no zsh.
    #[test]
    fn the_function_is_valid_zsh() {
        if !zsh_here() {
            eprintln!("no zsh here: skipped");
            return;
        }
        let d = scratch("syntax");
        let file = d.join("_ift");
        std::fs::write(&file, ZSH).unwrap();
        let out = std::process::Command::new("zsh")
            .arg("-n")
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// join and connect complete hosts the ssh way, not from /etc/hosts only.
    #[test]
    fn join_and_connect_take_the_ssh_hosts() {
        for verb in ["join) _arguments", "connect) _arguments"] {
            let line = ZSH.lines().find(|l| l.contains(verb)).unwrap();
            assert!(line.contains("'1:host:_ift_hosts'"), "{line}");
        }
    }

    /// Runs `_ift_ssh_hosts` in a real zsh against a made-up home.
    #[test]
    fn hosts_come_from_ssh_config_what_it_includes_and_known_hosts() {
        if !zsh_here() {
            eprintln!("no zsh here: skipped");
            return;
        }
        let home = scratch("hosts");
        let ssh = home.join(".ssh");
        std::fs::create_dir_all(ssh.join("extra")).unwrap();
        std::fs::write(
            ssh.join("config"),
            "Host mini air\n  HostName 100.1.1.1\nHOST upper-host\n\
             Host *.internal ci-? !not-this\n\
             Include extra/*.conf\n\
             Include ~/more.conf\n\
             Include ~/.ssh/config\n\
             Include /no/such/file\n",
        )
        .unwrap();
        std::fs::write(ssh.join("extra").join("a.conf"), "Host from-include\n").unwrap();
        std::fs::write(home.join("more.conf"), "Host from-tilde\n").unwrap();
        std::fs::write(
            ssh.join("known_hosts"),
            "|1|hashed== ssh-rsa AAAA\n\
             known.example,192.168.1.9 ssh-ed25519 AAAA\n\
             [bracket.example]:2222 ssh-ed25519 AAAA\n\
             # a comment\n\
             @cert-authority *.corp ssh-rsa AAAA\n\
             mini ssh-ed25519 AAAA\n",
        )
        .unwrap();
        let functions: String = ZSH
            .lines()
            .filter(|l| *l != "_ift \"$@\"")
            .collect::<Vec<_>>()
            .join("\n");
        let run = |call: &str| -> Vec<String> {
            let script = home.join("run.zsh");
            std::fs::write(&script, format!("{functions}\n{call}\n")).unwrap();
            let out = std::process::Command::new("zsh")
                .arg("-f")
                .arg(&script)
                .env("HOME", &home)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let mut got: Vec<String> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::to_string)
                .collect();
            got.sort();
            got
        };
        let mut aliases = vec!["mini", "air", "upper-host", "from-include", "from-tilde"];
        aliases.sort_unstable();
        assert_eq!(
            run("_ift_ssh_hosts"),
            aliases,
            "wildcards and negations are left out, includes are followed once"
        );
        // Known hosts: names only. The hashed entry, the comment, the CA line
        // and the bare IP address are left out, a bracketed host loses its port.
        let mut known = vec!["known.example", "bracket.example", "mini"];
        known.sort_unstable();
        assert_eq!(run("_ift_known_hosts"), known);
        let _ = std::fs::remove_dir_all(&home);
    }
}
