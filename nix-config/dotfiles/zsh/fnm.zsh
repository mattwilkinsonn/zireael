# fnm — initialise per shell, NOT via _evalcache: `fnm env` mints a fresh
# multishell symlink and exports its path on each call, so caching pins every
# shell to the first one and `fnm use` / --use-on-cd then leak across panes.
if command -v fnm >/dev/null; then
	eval "$(fnm env --use-on-cd --shell zsh)"

	# direnv restores PATH from its saved diff during chpwd. A nested shell can
	# inherit an active direnv diff from its parent, then fnm creates a fresh
	# multishell path during zsh startup; on the next cd, direnv's restore removes
	# that fresh fnm path before fnm's own use-on-cd hook checks PATH. Repair the
	# active multishell entry immediately before fnm's hook runs.
	_fnm_path_repair() {
		[[ -n ${FNM_MULTISHELL_PATH:-} ]] || return 0
		local fnm_bin="$FNM_MULTISHELL_PATH/bin"
		[[ -d "$fnm_bin" ]] || return 0
		local -a repaired_path
		local entry
		local inserted=0
		for entry in "${path[@]}"; do
			if [[ "$entry" == "$fnm_bin" ]]; then
				if (( ! inserted )); then
					repaired_path+=("$entry")
					inserted=1
				fi
				continue
			fi
			if [[ "$entry" == */fnm_multishells/*/bin ]]; then
				if (( ! inserted )); then
					repaired_path+=("$fnm_bin")
					inserted=1
				fi
				continue
			fi
			repaired_path+=("$entry")
		done
		if (( ! inserted )); then
			repaired_path+=("$fnm_bin")
		fi
		path=("${repaired_path[@]}")
		export PATH
	}

	chpwd_functions=(${chpwd_functions:#_fnm_path_repair})
	if (( ${chpwd_functions[(Ie)_fnm_autoload_hook]} )); then
		chpwd_functions=(${chpwd_functions:#_fnm_autoload_hook} _fnm_path_repair _fnm_autoload_hook)
	else
		autoload -U add-zsh-hook
		add-zsh-hook chpwd _fnm_path_repair
	fi
	_fnm_path_repair
fi
