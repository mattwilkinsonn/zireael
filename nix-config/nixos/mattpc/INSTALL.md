# mattpc — bare-metal dual-boot install

Turns the gaming PC into a **bare-metal NixOS daily driver** dual-booting
Windows (games only). NixOS takes **Disk 0** (the 2 TB Samsung 980 PRO that
held the WSL `ext4.vhdx`); **Windows stays on Disk 1, untouched**.

## Hardware (confirmed from the running WSL host)

| Disk | Serial (Windows-reported) | Was | Becomes |
| --- | --- | --- | --- |
| **0** | `0025_38BA_21A1_303A` | `G:` — 99% the WSL vhdx | **NixOS** (wiped) |
| **1** | `0025_38B2_4140_9DA2` | `C:` Windows + 200 MB ESP | **untouched** |

Both are the **identical model**, so the installer targets Disk 0 by
`/dev/disk/by-id` (serial), never `/dev/nvmeXn1`.

Layout (disko): 1 GiB ESP (vfat `/boot`) · 64 GiB swap (= RAM, hibernate) ·
btrfs root with `@` → `/`, `@home` → `/home`, `@nix` → `/nix` (zstd).

## 0. Pre-flight (before touching the disk)

1. **Push every agent's work** and anything uncommitted in the WSL distro —
   the wipe destroys the WSL distro on Disk 0.
2. Confirm nothing else on `G:` matters (it's ~0.3 GB beyond the vhdx).
3. Have a NixOS installer USB (any recent unstable ISO) + this repo reachable.

## 1. Boot the installer + identify Disk 0

Boot the NixOS installer USB. Identify the target by **elimination** (the
serials look alike, so verify by content):

```bash
lsblk -f          # find the two nvme disks
ls -l /dev/disk/by-id/ | grep -i nvme
```

- **Disk 1 (Windows — DO NOT TOUCH):** the NVMe with a ~200 MB vfat ESP + a
  large NTFS partition (Windows `C:`).
- **Disk 0 (target):** the NVMe with a single large NTFS labelled `Games` and
  nothing else.

Copy Disk 0's stable path, e.g. `/dev/disk/by-id/nvme-Samsung_SSD_980_PRO_2TB_<serial>`.

## 2. Fill the two install-time placeholders

Clone the repo in the installer, then edit:

- `nixos/mattpc/disko.nix` → set `device` to Disk 0's by-id path (replaces
  `REPLACE-WITH-DISK0-BY-ID`).

The Windows boot entry's `efiDeviceHandle` is the second placeholder
(`REPLACE-WITH-EDK2-FS-HANDLE` in `system.nix`), but it can't be filled until
first boot — you discover it from the edk2 UEFI Shell (§6). Leave it as-is for
now; the config evaluates with the placeholder, the entry just won't chainload
until you set the real handle.

## 3. Partition + format + mount (disko)

```bash
# DESTROYS Disk 0. Double-check the device is Disk 0, not Windows.
sudo nix --experimental-features "nix-command flakes" run github:nix-community/disko -- \
  --mode disko ./nixos/mattpc/disko.nix
```

disko partitions, formats (btrfs subvols + swap + ESP), and mounts everything
under `/mnt`.

## 4. Reconcile hardware config

```bash
sudo nixos-generate-config --no-filesystems --root /mnt
```

`--no-filesystems` keeps disko as the single source of the mount layout. Diff
the generated `/mnt/etc/nixos/hardware-configuration.nix` against this repo's
`nixos/mattpc/hardware-configuration.nix` and merge any extra
`boot.initrd.availableKernelModules` the scan found.

## 5. Install

```bash
sudo nixos-install --flake /mnt/etc/nixos-repo/nix-config#mattpc
# (point --flake at wherever you cloned the repo)
reboot
```

Set the user password on first login (or via `nixos-install`'s prompt).

## 6. Discover the Windows ESP handle, then the SSH boot-select

systemd-boot is the default (boots NixOS). Windows Boot Manager lives on Disk
1's ESP; since that's a *different* disk, systemd-boot chainloads it through the
bundled edk2 UEFI Shell. The one-time discovery of Disk 1's ESP handle:

1. Reboot and pick **EDK2 UEFI Shell** from the systemd-boot menu.
2. `map -c` — lists the consistent filesystem handles (`FS0`, `FS1`, …).
3. For each, check for the Windows bootloader at
   `FS1:\EFI\Microsoft\Boot\bootmgfw.efi`.
   Run `ls FS1:\EFI` first; the handle whose tree contains
   `Microsoft\Boot\bootmgfw.efi` is Disk 1's ESP. (Running the `.efi` path
   directly boots Windows — a good confirmation.)
4. Set that handle in `system.nix`:
   `boot.loader.systemd-boot.windows."windows".efiDeviceHandle = "FS1";`
   (replacing `REPLACE-WITH-EDK2-FS-HANDLE`), then `nix-switch` (§7).

Now **Windows** is a normal entry in the systemd-boot menu. To switch **over
SSH** without touching the console:

```bash
bootctl list                              # confirm the "Windows" entry exists
sudo bootctl set-oneshot windows          # next boot only → Windows
sudo reboot
# …after the gaming session, a normal Windows restart returns to NixOS:
# set-oneshot is consumed on that single boot, so the default (NixOS) resumes.
```

That's the remote OS-selection mechanism: `bootctl set-oneshot windows && reboot`
from anywhere on the tailnet. Default stays NixOS; Windows is a one-shot.

- **Manual fallback:** the systemd-boot menu at power-on lists NixOS, Windows,
  and the EDK2 UEFI Shell; the firmware menu (F-key) lists both disks directly.
- **Persistent default swap** (rarely needed): `bootctl set-default windows`
  (or a NixOS generation); `bootctl set-default @saved` clears it.

## 7. Converge afterward

`nix-switch` on this host is aliased to
`sudo nixos-rebuild switch --flake ~/repos/zireael/nix-config#mattpc`.
