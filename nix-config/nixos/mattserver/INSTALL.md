# mattserver — NixOS Install Procedure

Old gaming PC repurposed as a ZFS backup target, Buildkite CI agent host,
and on-demand gaming station.

**Hardware:**

- AMD Ryzen 3600 6-Core
- B450 Tomahawk (UEFI)
- 64 GB DDR4
- PowerColor RX 5700 XT 8 GB (RDNA 1 / Navi 10)
- HP EX920 1 TB M.2 NVMe → btrfs root
- Seagate FireCuda 2 TB SATA SSHD → ZFS backup pool

## Pre-install checklist

- [ ] `nix-config` repo pushed (run `config status` on the Mac to confirm).
- [ ] Tailscale pre-auth key ready at <https://login.tailscale.com/admin/settings/keys>.
      Single-use, tagged `tag:server,tag:ci-runner` (mattserver also gets
      `tag:redis` for the sccache L1 — assign it after enrolment, or add it
      to the key's tags too).
- [ ] Buildkite agent token ready (org Agents page → Reveal Agent Token; see
      "Buildkite agent token" section below).
- [ ] New `mattw` + `root` password ready to type at the bootstrap prompt
      (manually rotated — see "Security posture" below for why this isn't 1P-driven).
- [ ] Ventoy stick has the latest **NixOS 25.11 minimal or graphical ISO**
      from <https://nixos.org/download> (x86\_64-linux).

## BIOS settings

Power on → Delete (or F2) to enter BIOS:

1. **Secure Boot: Off.** NixOS's bootloader is not signed.
2. **Boot order: Ventoy USB first.**
3. CSM / Legacy boot: disable if present (systemd-boot requires UEFI).

Save and exit.

## Boot the installer

Insert Ventoy stick, F12 for boot menu, select Ventoy, pick the NixOS ISO.
From a live TTY or Konsole, run everything below.

Get on the network first:

```bash
ping 1.1.1.1
```

## Partition layout

Two physical disks. The NVMe gets the btrfs root; the SSHD gets the ZFS pool.
Do them separately.

### Identify disks

```bash
lsblk -d
```

Typical layout:

- NVMe root: `/dev/nvme0n1`
- SATA SSHD: `/dev/sda`

Set variables — **confirm before continuing**:

```bash
NVME=/dev/nvme0n1
SSHD=/dev/sda
```

### NVMe — btrfs root

```bash
sudo parted "$NVME" -- mklabel gpt
sudo parted "$NVME" -- mkpart ESP fat32 1MiB 1025MiB
sudo parted "$NVME" -- set 1 esp on
sudo parted "$NVME" -- mkpart swap linux-swap 1025MiB 17409MiB
sudo parted "$NVME" -- mkpart root btrfs 17409MiB 100%
```

```bash
ESP="${NVME}p1"
SWAP="${NVME}p2"
ROOT="${NVME}p3"
```

Format:

```bash
sudo mkfs.fat -F 32 -n BOOT "$ESP"
sudo mkswap -L swap "$SWAP"
sudo mkfs.btrfs -f -L nixos "$ROOT"
```

Create subvolumes:

```bash
sudo mount "$ROOT" /mnt
sudo btrfs subvolume create /mnt/@
sudo btrfs subvolume create /mnt/@home
sudo btrfs subvolume create /mnt/@nix
sudo btrfs subvolume create /mnt/@log
sudo umount /mnt
```

Mount for nixos-install:

```bash
BTRFS_OPTS="compress=zstd:1,noatime,ssd,discard=async"

sudo mount -o "subvol=@,$BTRFS_OPTS" "$ROOT" /mnt
sudo mkdir -p /mnt/{home,nix,var/log,boot}
sudo mount -o "subvol=@home,$BTRFS_OPTS" "$ROOT" /mnt/home
sudo mount -o "subvol=@nix,$BTRFS_OPTS" "$ROOT" /mnt/nix
sudo mount -o "subvol=@log,$BTRFS_OPTS" "$ROOT" /mnt/var/log
sudo mount "$ESP" /mnt/boot
sudo swapon "$SWAP"
```

> **SATA SSHD — ZFS backup pool:** the NixOS minimal installer image ships
> without ZFS userspace tools (CDDL vs GPL licensing), so `zpool` is not
> available here. Pool creation is deferred to after first boot — the
> installed system has `boot.supportedFilesystems = [ "zfs" ]` from
> `system.nix`, so `zpool` works natively post-reboot. See the
> [ZFS backup pool setup](#zfs-backup-pool-setup) section below.

## Run the installer

Copy the flake from Ventoy and run nixos-install:

```bash
mkdir -p /tmp/flake
cp -r /run/media/*/Ventoy/home-manager/. /tmp/flake/

sudo nixos-generate-config --root /mnt --dir /tmp/nixos-gen
cat /tmp/nixos-gen/hardware-configuration.nix
# Check initrd.availableKernelModules for anything not covered by the
# ZFS-pinned kernel (system.nix uses pkgs.zfs.latestCompatibleLinuxPackages).

sudo nixos-install --flake /tmp/flake#mattserver --no-root-passwd
```

Reboot and pull the Ventoy stick during POST:

```bash
sudo reboot
```

## First boot

Login as `mattw` (initial password from `nixos/common.nix`). Run the
bootstrap from the Ventoy stick (dotfiles haven't been cloned yet):

```bash
ls /run/media/*/Ventoy/home-manager/nixos/scripts/mattserver-bootstrap.sh
bash /run/media/*/Ventoy/home-manager/nixos/scripts/mattserver-bootstrap.sh
```

## Security posture

mattserver runs untrusted CI workflows via the self-hosted Buildkite
agent pool. The threat model assumes that a malicious workflow could
land code-exec as a `buildkite-agent-*` user — already mitigated
upstream by external-PR approval gates + dep-update cooldowns, but
defense-in-depth on the host shrinks the blast radius if those upstream
layers fail.

The deliberate posture choices on this host:

- **Zero standing 1Password service-account tokens.** No personal SA,
  no team SA. The bootstrap script (step 4) prompts for the user
  password manually rather than fetching from 1P. Any SA token on disk
  is something a compromised agent could exfiltrate; the simplest
  answer is "have no SA tokens." The documented downside is that
  password rotation is operator-driven (run the bootstrap script and
  type the new password); the upside is the agent has zero capability
  against the 1P account.
- **Agent token encrypted at rest** via systemd-creds (host-bound
  machine-ID encryption). The encrypted file at
  `/etc/buildkite-agent/agent-token.cred` can't be decrypted off this
  box; the plaintext only lives in tmpfs after
  `decrypt-agent-token.service` runs at boot. See "Buildkite agent
  token" below.
- **No inter-server SSH keypair.** Earlier configs minted a shared
  `inter-server` ed25519 key in 1P and provisioned the private half
  to every NixOS host for cross-host automation. Retired — the only
  consumer (Pi-to-mattserver ZFS backups) wasn't actually wired up,
  and the standing private key was a credential a compromised agent
  could reach. The matching `authorizedKeys` line is gone from
  `nixos/common.nix`. If we ever set up cross-host automation
  again, mint a fresh per-pair keypair scoped to the specific
  endpoints.
- **No native sshd on the LAN side.** `services.openssh.enable =
  lib.mkForce false` on this host (overrides the common.nix
  default). Tailscale SSH covers every interactive access path;
  the LAN-side sshd was unneeded attack surface.
- **No Cockpit.** `services.cockpit.enable = lib.mkForce false` —
  mattserver inherits cockpit from common.nix but doesn't
  tailscale-serve it, so the only path was LAN inbound on :9090.
  Replaced operationally by Tailscale SSH + `btm` for live
  monitoring.
- **No host-level egress lockdown on the agent (yet).** The
  pre-SEA-830 runner setup dropped the runner UID's outbound to
  RFC1918 / link-local / CGNAT (LAN-pivot defense-in-depth). That
  rule relied on jobs running as forks of the runner UID in the host
  netns; SEA-830 moved PR jobs into a podman container, where a
  GID-by-`output` rule both mis-scopes (supplementary group) and
  misses the hook (container egress is `forward`/`postrouting`). It
  was dropped rather than shipped failing-open. Container-aware LAN
  egress isolation is tracked in **SEA-835**. (mattmini runs jobs
  natively, so its pf egress rule is still correctly scoped.)

## Buildkite agent token

> The agents register with the Buildkite **Agent** token (org Agents
> page → Reveal Agent Token) — distinct from the `BUILDKITE_API_TOKEN`
> the `bk` CLI uses. The encrypted file at
> `/etc/buildkite-agent/agent-token.cred` must exist before the
> `buildkite-agent-*` units start, or they fail with no token to
> decrypt. The rest of the host works without it if you're not wiring
> up the agents yet.

The two agent instances (`sealed`, `sealed-2`) share a single
org-scoped agent token. The plaintext is encrypted at rest via
`systemd-creds` (host-bound — only this box's machine ID can decrypt)
and decrypted into tmpfs (`/run/buildkite-agent/agent-token`) at boot
by `decrypt-agent-token.service`.

Get the agent token from
<https://buildkite.com/organizations/sealedsecurity/agents> (Reveal
Agent Token). Then encrypt it with the helper script (prompts for the
token, writes the host-bound .cred file):

```bash
sudo bash ~/repos/zireael/nix-config/nixos/scripts/mattserver-encrypt-agent-token.sh
```

The script verifies the decrypt path before exiting, so a successful
run means the agent units will pick up the same plaintext at boot.

> **Why systemd-creds instead of a plaintext token file:** the .cred
> file is decryptable only by this host (encryption is bound to the
> machine ID), so an attacker who exfiltrates `/etc/buildkite-agent/`
> off the box gets a useless blob. The plaintext only ever lives in
> tmpfs after decrypt — it disappears on shutdown. Same pattern the
> Pi uses for `op-pi-svc-token.cred`.

Rebuild — both agent services come up and register:

```bash
nix-switch
sudo systemctl status \
  decrypt-agent-token \
  buildkite-agent-sealed \
  buildkite-agent-sealed-2
```

Confirm the two agents appear at
<https://buildkite.com/organizations/sealedsecurity/agents> with the
tag `queue=linux-x64-selfhosted`.

To rotate the token, re-run the encrypt script with the new value and
restart the decrypt + agent units:

```bash
sudo bash ~/repos/zireael/nix-config/nixos/scripts/mattserver-encrypt-agent-token.sh
sudo systemctl restart decrypt-agent-token.service
sudo systemctl restart 'buildkite-agent-sealed*.service'
```

To later expand the pool (sealed-3, etc.), add an entry to
`services.buildkite-agents` in `nixos/mattserver/system.nix` plus a
matching `systemd.services.buildkite-agent-<name>` decrypt-ordering
stanza, then rebuild — no token changes are needed because the
org-level agent token registers any number of agents.

## ZFS backup pool setup

After first boot, create the backup pool on the SATA SSHD. Single-disk pool
named `tank`. The SSHD's hybrid SSD cache layer is firmware-managed; no
special ZFS options needed.

Re-confirm the disk before creating the pool:

```bash
lsblk -d
SSHD=/dev/sda
```

Create the pool:

```bash
sudo zpool create -f \
  -o ashift=12 \
  -O compression=lz4 \
  -O atime=off \
  -O xattr=sa \
  tank "$SSHD"
```

Create the receive dataset that source hosts will send into:

```bash
sudo zfs create tank/backups
```

Verify:

```bash
sudo zpool status tank
sudo zfs list
```

The pool will be auto-imported on every subsequent boot via the `hostId`
set in `system.nix` (`0bf374c7`).

## ZFS backup receive setup

Grant the `backup` user permission to receive ZFS snapshots into
`tank/backups` without a root shell (one-time, survives reboots because
ZFS stores delegation in pool metadata):

```bash
sudo zfs allow -u backup create,destroy,mount,mountpoint,receive,rollback,bookmark tank/backups
```

Verify the delegation:

```bash
sudo zfs allow tank/backups
```

### Sending backups from a source host

Not currently wired up. The `backup` user exists with the ZFS
delegation in place, but `authorizedKeys = [ ]` — no source host
can SSH in to push receives yet.

When wiring this back up:

1. Generate a dedicated keypair on the source host (don't reuse keys
   from other automation — minimal blast radius if any one host is
   compromised):

   ```bash
   ssh-keygen -t ed25519 -C 'rpi5-to-mattserver-backup' \
     -f ~/.ssh/id_ed25519_backup_rpi5
   ```

2. Add the public key to `nixos/mattserver/system.nix` under
   `users.users.backup.openssh.authorizedKeys.keys`, scoped per
   source host (one entry per Pi) with a comment naming the source.
3. `nix-switch` on mattserver to install.
4. Push via syncoid on the source host:

   ```bash
   syncoid --sshkey ~/.ssh/id_ed25519_backup_rpi5 \
     tank/data \
     backup@mattserver.tail08a5c5.ts.net:tank/backups/rpi5
   ```

   Tailscale hostname is preferred over the LAN IP — the inbound
   firewall on mattserver only trusts `tailscale0`, and future
   hardening (e.g. dropping LAN inbound entirely) won't break the
   path.

## Gaming

Boots straight to SDDM by default — pick your user and log in. Steam is
available immediately after login; first-time setup requires accepting the
Steam Runtime license and letting it download the runtime layer (~300 MB).
Proton is downloaded per-game on first launch.

To temporarily drop to a headless session without rebuilding:

```bash
sudo systemctl isolate multi-user.target  # this boot only
sudo systemctl isolate graphical.target   # back to the DE
```

To make headless the default again, set `bootToDesktop = false;` in
`nixos/mattserver/desktop.nix` and `nix-switch`.

## SSH from the Mac

Add to `~/.ssh/config` (Mac side):

```text
Host mattserver
  HostName mattserver.tail08a5c5.ts.net
  User mattw
  IdentityAgent ~/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock
```
