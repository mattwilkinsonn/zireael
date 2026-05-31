# mattserver — NixOS Install Procedure

Old gaming PC repurposed as a ZFS backup target, GitHub Actions runner host,
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
- [ ] Personal 1Password service-account token ready (read access to Dev + Server
      vaults).
- [ ] Team 1Password service-account token ready (read access to Employee Dev
      vault on sealedsecurity.1password.com).
- [ ] GitHub PAT ready for the sealed runner pool (see GitHub runner token section below).
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

## GitHub runner token

> **Runners are disabled by default** (`enableRunners = false` at the top
> of `nixos/mattserver/system.nix`). The token-write + flip-and-rebuild
> step below is what turns them on. Skip this section entirely if you're
> not ready to wire up runners yet — the rest of the host works without
> them.

The four runner instances (`sealed`, `sealed-2`, `sealed-3`, `sealed-4`)
share a single org-scoped token. The plaintext PAT is encrypted at rest
via `systemd-creds` (host-bound — only this box's machine ID can
decrypt) and decrypted into tmpfs (`/run/github-runner/sealed-token`)
at boot by `decrypt-runner-token.service`. The encrypted file at
`/etc/github-runner/sealed-token.cred` must exist *before*
`enableRunners` flips to `true` or `nixos-rebuild` will fail trying to
start services with no token to decrypt.

Token scope:

| Runner pool | Encrypted file | PAT scope |
| ----------- | -------------- | --------- |
| `sealed`, `sealed-2`, `sealed-3`, `sealed-4` | `/etc/github-runner/sealed-token.cred` | `manage_runners:org` (fine-grained) or `admin:org` (classic) |

Create a fine-grained PAT scoped to the sealedsecurity org at
<https://github.com/organizations/sealedsecurity/settings/personal-access-tokens>.
Then encrypt it with the helper script (prompts for the token, writes
the host-bound .cred file, optionally shreds any pre-existing
plaintext):

```bash
sudo bash ~/repos/zireael/nix-config/nixos/scripts/mattserver-encrypt-runner-token.sh
```

The script verifies the decrypt path before exiting, so a successful
run means the runner units will pick up the same plaintext at boot.

> **Why systemd-creds instead of a plaintext token file:** the .cred
> file is decryptable only by this host (encryption is bound to the
> machine ID), so an attacker who exfiltrates `/etc/github-runner/`
> off the box gets a useless blob. The plaintext only ever lives in
> tmpfs after decrypt — it disappears on shutdown. Same pattern the
> Pi uses for `op-pi-svc-token.cred`.

Now flip `enableRunners = true;` in `nixos/mattserver/system.nix` and
rebuild — all four runner services come up registered:

```bash
nix-switch
sudo systemctl status \
  decrypt-runner-token \
  github-runner-sealed \
  github-runner-sealed-2 \
  github-runner-sealed-3 \
  github-runner-sealed-4
```

Confirm the four runners appear at
<https://github.com/organizations/sealedsecurity/settings/actions/runners>
with labels `[self-hosted, Linux, X64, seal-linux-x64, mattserver]`.

If you're migrating from an earlier plaintext layout, clean up the
leftover files:

```bash
sudo shred -u /etc/github-runner/sealed-token         # plaintext PAT
sudo rm -f   /etc/github-runner/personal-token        # pre-2026 split
```

To rotate the PAT, re-run the encrypt script with the new token and
restart the decrypt + runner units:

```bash
sudo bash ~/repos/zireael/nix-config/nixos/scripts/mattserver-encrypt-runner-token.sh
sudo systemctl restart decrypt-runner-token.service
sudo systemctl restart 'github-runner-sealed*.service'
```

To later expand the pool (sealed-5, etc.), edit
`nixos/mattserver/system.nix` and rebuild — no token changes are needed
because the org-level PAT registers any number of runners against the
same scope.

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

On the source host (e.g. rpi5), use syncoid with the inter-server SSH key:

```bash
syncoid --sshkey ~/.ssh/id_ed25519_inter_server \
  tank/data \
  backup@mattserver:tank/backups/rpi5
```

Or via Tailscale hostname:

```bash
syncoid --sshkey ~/.ssh/id_ed25519_inter_server \
  tank/data \
  backup@mattserver.tail08a5c5.ts.net:tank/backups/rpi5
```

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
