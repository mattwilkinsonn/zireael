{ ... }:

# mattmini — owned Apple Silicon Mac mini (M2 Pro, 16 GB, 512 GB), a
# native macOS arm64 Buildkite runner for the sealedsecurity org. The
# whole agent surface lives in the shared
# darwin/modules/buildkite-agent-macos.nix module; only the per-host
# knobs are here.
#
# cargoBuildJobs = 6: the M2 Pro is 6 performance cores (10-core
# variant) — keeps heavy codegen on the P-cores. Bump to 8 if this is
# the 8P 12-core M2 Pro (verify: sysctl -n hw.perflevel0.physicalcpu).
#
# adminUser defaults to `mattw` (created during macOS install).

{
  imports = [ ../modules/buildkite-agent-macos.nix ];

  sealed.macosBuildkiteAgent = {
    enable = true;
    hostName = "mattmini";
    cargoBuildJobs = 6;
  };
}
