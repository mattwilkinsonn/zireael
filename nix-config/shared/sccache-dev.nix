{ lib, ... }:
{
  # Rust compiler cache. sccache reads its entire configuration from the
  # environment (v0.15.0; env config overrules any file), so there is no
  # generated config file to manage. These are the always-on local defaults;
  # load-secrets exports the Redis/R2 secret vars plus the multi-level chain
  # once 1Password is available, otherwise sccache uses the disk level only.
  programs.zsh.profileExtra = lib.mkBefore ''
    export RUSTC_WRAPPER=sccache
    export SCCACHE_DIR="$HOME/.cache/sccache"
    export SCCACHE_CACHE_SIZE=10G
    export SCCACHE_REGION=auto
    export SCCACHE_S3_USE_SSL=true
  '';
}
