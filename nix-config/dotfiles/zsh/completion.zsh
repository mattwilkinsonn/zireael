# Fast compinit: skip the ~280ms security audit + redump on shells where
# nothing changed. Rebuild only when the dump is missing, the active nix
# generation is newer than the dump (a nix-switch changed the completion files
# in fpath), or the dump is >24h old as a backstop. zstat -L reads
# /run/current-system's *symlink* mtime (= last switch); the store path it
# resolves to is epoch-stamped and useless for an mtime compare. home-manager
# runs as a system module here, so /run/current-system bumps on every switch.
autoload -Uz compinit
zmodload -F zsh/stat b:zstat 2>/dev/null
() {
  emulate -L zsh
  setopt extended_glob
  local dump=${ZDOTDIR:-$HOME}/.zcompdump
  local -a ds ss
  zstat -A ds +mtime -- $dump 2>/dev/null
  zstat -L -A ss +mtime -- /run/current-system 2>/dev/null
  if [[ ! -e $dump ]] || (( ${ss[1]:-0} > ${ds[1]:-0} )) || [[ -n $dump(#qN.mh+24) ]]; then
    compinit -i -d $dump
  else
    compinit -C -d $dump
  fi
}
