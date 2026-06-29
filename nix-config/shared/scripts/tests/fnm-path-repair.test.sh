#!/usr/bin/env bash
# Regression test for keeping fnm's active multishell on PATH after direnv rewrites PATH.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NIX_CONFIG_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
FNM_INIT="$NIX_CONFIG_DIR/dotfiles/zsh/fnm.zsh"

tmp_root="$(mktemp -d)"
tmp="$tmp_root/fnm[glob]"
trap 'rm -rf "$tmp_root"' EXIT

mkdir -p "$tmp/bin" "$tmp/current/bin" "$tmp/stale/bin" "$tmp/target"
touch "$tmp/target/package.json"

cat >"$tmp/bin/fnm" <<'FNM'
#!/usr/bin/env bash
set -euo pipefail

case "${1:-}" in
	env)
		cat <<'ZSH'
export PATH="$FNM_MULTISHELL_PATH/bin":$PATH
autoload -U add-zsh-hook
_fnm_autoload_hook () {
	if [[ -f .node-version || -f .nvmrc || -f package.json ]]; then
		fnm use --silent-if-unchanged
	fi
}
add-zsh-hook -D chpwd _fnm_autoload_hook
add-zsh-hook chpwd _fnm_autoload_hook
ZSH
		;;
	use)
		case ":$PATH:" in
			*":$FNM_MULTISHELL_PATH/bin:"*) ;;
			*) echo "warning: The current Node.js path is not on your PATH environment variable." >&2 ;;
		esac
		;;
esac
FNM
chmod +x "$tmp/bin/fnm"

cat >"$tmp/case.zsh" <<'ZSH'
set -e

autoload -Uz add-zsh-hook
setopt GLOB_SUBST
path=("$TMP/bin" "$TMP/stale/bin" $path)
export PATH
export FNM_MULTISHELL_PATH="$TMP/current"

_direnv_hook() {
	local -a restored_path
	local entry
	restored_path=("$TMP/stale/bin")
	for entry in "${path[@]}"; do
		[[ "$entry" == "$TMP/current/bin" ]] && continue
		restored_path+=("$entry")
	done
	path=("${restored_path[@]}")
	export PATH
}

chpwd_functions=(_direnv_hook)
source "$FNM_INIT"
cd "$TMP/target"
print -r -- "PATH=$PATH"
print -r -- "CURRENT_BIN=$TMP/current/bin"
ZSH

status=0
out="$(TMP="$tmp" FNM_INIT="$FNM_INIT" zsh -f "$tmp/case.zsh" 2>&1)" || status=$?

if [ "$status" -ne 0 ]; then
	printf 'FAIL zsh fixture exited %s\n%s\n' "$status" "$out"
	exit "$status"
fi

case "$out" in
*"warning: The current Node.js path is not on your PATH environment variable."*)
	printf 'FAIL fnm warned after direnv PATH rewrite\n%s\n' "$out"
	exit 1
	;;
esac

case "$out" in
*"$tmp/current/bin"*) ;;
*)
	printf 'FAIL current fnm multishell bin missing after cd\n%s\n' "$out"
	exit 1
	;;
esac

current_bin="$(printf '%s\n' "$out" | while IFS= read -r line; do case "$line" in CURRENT_BIN=*) printf '%s' "${line#CURRENT_BIN=}" ;; esac done)"
path_line="$(printf '%s\n' "$out" | while IFS= read -r line; do case "$line" in PATH=*) printf '%s' "${line#PATH=}" ;; esac done)"
current_count=0
old_ifs=$IFS
IFS=:
for entry in $path_line; do
	if [ "$entry" = "$current_bin" ]; then
		current_count=$((current_count + 1))
	fi
done
IFS=$old_ifs

if [ "$current_count" -ne 1 ]; then
	printf 'FAIL current fnm multishell bin appears %s times\n%s\n' "$current_count" "$out"
	exit 1
fi

printf 'ok fnm multishell survives direnv PATH rewrite\n'
