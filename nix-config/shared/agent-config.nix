{ config, ... }:

# shared/agent-config.nix — tool-agnostic agent config, canonical in this repo
# under agents/ and symlinked into $HOME. Split by which tool reads what:
#   - ~/.agents/{AGENTS.md,rules,skills}: read by OMP's .agent[s] discovery
#     provider (coding-agent discovery/agents.ts) and by any other
#     .agents-aware CLI — so they live tool-agnostic under ~/.agents.
#     OMP resolves AGENTS.md / rules / skills ONLY from here — there is
#     intentionally no ~/.omp/agent/{AGENTS.md,rules,skills}, so a review that
#     flags their absence under ~/.omp/agent as "legacy paths missing" is
#     mistaken: ~/.agents IS the live location (per the .agents discovery above).
#   - ~/.omp/agent/{extensions,mcp.json,config.yml}: OMP-specific (read by the
#     native omp provider; config.yml is also written back, in-place, so the
#     write follows the symlink). Not part of .agents discovery, so they stay
#     under ~/.omp/agent. agent.db + sessions stay OMP-managed live (not linked).
#
# mkOutOfStoreSymlink (not source = ./agents/...): agents edit AGENTS.md / rules
# / skills in place, so the link targets the live working copy and edits take
# effect without a rebuild. The target is a runtime path, so these files need
# not be git-tracked for the flake to evaluate.
#
# Imported by dev hosts that have nix-config checked out at
# ~/repos/zireael/nix-config (alongside privatefiles-symlinks.nix).

let
  agents = "${config.home.homeDirectory}/repos/zireael/nix-config/agents";
  linkAgent = path: config.lib.file.mkOutOfStoreSymlink "${agents}/${path}";
in
{
  home.file = {
    ".agents/AGENTS.md".source = linkAgent "AGENTS.md";
    ".agents/rules".source = linkAgent "rules";
    ".agents/skills".source = linkAgent "skills";
    ".omp/agent/extensions".source = linkAgent "extensions";
    ".omp/agent/mcp.json".source = linkAgent "mcp.json";
    ".omp/agent/config.yml".source = linkAgent "config.yml";
  };
}
