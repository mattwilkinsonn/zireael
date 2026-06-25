# Fast compinit: skip the security audit + full redump on every shell.
# Full audit/redump only when the dump is missing or >24h old; otherwise
# load the cached dump with -C (skips the ~280ms audit + compdump).
autoload -Uz compinit
() {
  emulate -L zsh
  setopt extended_glob
  local dump=${ZDOTDIR:-$HOME}/.zcompdump
  if [[ -n $dump(#qN.mh+24) || ! -e $dump ]]; then
    compinit -i -d $dump
  else
    compinit -C -d $dump
  fi
}
