{ ... }:

# awsmac — AWS EC2 mac2-m2.metal (Apple M2, 8-core, 24 GB), a STOPGAP
# macOS arm64 Buildkite runner billed hourly on AWS credits while the
# owned mattmini / dedicatedmac.io rental come online. The whole agent
# surface lives in the shared darwin/modules/buildkite-agent-macos.nix
# module; only the per-host knobs are here.
#
# adminUser = ec2-user: the AWS macOS AMI ships that admin account
# already (passwordless sudo, SSH keypair wired), so we apply
# home-manager to it rather than creating `mattw`.
#
# cargoBuildJobs = 4: the M2 in mac2-m2.metal is 8-core (4P + 4E); 4
# keeps codegen on the performance cores.
#
# Teardown is the revocation: `aws ec2 terminate-instances` + release
# the dedicated host destroys the volume (and the plaintext secrets)
# with it. See darwin/awsmac/INSTALL.md.

{
  imports = [ ../modules/buildkite-agent-macos.nix ];

  sealed.macosBuildkiteAgent = {
    enable = true;
    hostName = "awsmac";
    adminUser = "ec2-user";
    cargoBuildJobs = "4";
  };
}
