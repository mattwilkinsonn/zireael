{ ... }:

# dedicatedmacio-mini — RENTED Apple Silicon Mac mini (M4 base, 16 GB,
# from dedicatedmac.io), a STOPGAP macOS arm64 Buildkite runner while
# the owned mattmini is racked. Same `macos-arm64-selfhosted` queue as
# mattmini and awsmac (jobs route to whichever agent is free). The
# whole agent surface lives in the shared
# darwin/modules/buildkite-agent-macos.nix module; only the per-host
# knobs are here.
#
# cargoBuildJobs = 4: the base M4 mini is 10-core (4P + 6E); 4 keeps
# codegen on the performance cores. Verify with `sysctl -n
# hw.perflevel0.physicalcpu` and bump for an M4 Pro variant (8P).
#
# adminUser defaults to `mattw` (created during macOS install).
#
# RENTAL posture: the box lives in dedicatedmac.io's datacenter on a
# shared subnet we don't control, and the provider re-images on return.
# The wipe is the secret revocation — no token rotation needed. See
# darwin/dedicatedmacio-mini/INSTALL.md "Rental exit".

{
  imports = [ ../modules/buildkite-agent-macos.nix ];

  sealed.macosBuildkiteAgent = {
    enable = true;
    hostName = "dedicatedmacio-mini";
    cargoBuildJobs = 4;
  };
}
