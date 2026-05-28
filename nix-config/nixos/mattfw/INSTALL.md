# mattfw — Framework Desktop NixOS Install Procedure

One-time install procedure for the Framework Desktop **Ryzen AI Max+ 395 / 128 GB
LPDDR5X** mini PC. Primary use: **local LLM inference target**, accessed over
SSH. mattpc-wsl is the primary dev box; this one is tuned for "biggest model +
biggest context window the hardware can take." Secondary: occasional KDE
Plasma session if local debug is ever needed.

The Framework guide
([guides.frame.work/Guide/NixOS+on+the+Framework+Desktop/417](https://guides.frame.work/Guide/NixOS+on+the+Framework+Desktop/417))
boils down to "boot the official NixOS ISO and install" — there's no
hardware-specific image. This doc fills in the gaps: BIOS UMA setting for the
iGPU memory split, btrfs partition layout, post-install bootstrap.

## Pre-install checklist

- [ ] All in-flight repo work pushed to git.
- [ ] `nix-config` repo pushed.
- [ ] Tailscale pre-auth key generated at
      <https://login.tailscale.com/admin/settings/keys>.
- [ ] Personal 1Password service-account token ready (read access to Dev + Server
      vaults). Create at <https://my.1password.com/developer-tools/serviceaccounts>.
- [ ] Team 1Password service-account token ready (read access to Employee Dev
      vault on sealedsecurity.1password.com). Create at
      <https://sealedsecurity.1password.com/developer-tools/serviceaccounts>.
- [ ] `op://Dev/Framework Password` item exists in 1P (used for the
      post-install user + root password rotation).
- [ ] Ventoy stick has the latest **NixOS 25.11 graphical (Plasma)** ISO
      from <https://nixos.org/download>. The graphical ISO is the easier
      path during partitioning — you get GParted, a file manager, and a
      browser inside the live env. The choice of live-env DE has zero
      effect on the installed system.
- [ ] Dotfiles repo pushed (verify with `config status`).

## Install media

Ventoy stick (already in use for mattpc / rpi installers). Run the sync
helper from the Mac to mirror this repo onto the stick — the bootstrap
script reads from there post-install:

```bash
bash ~/repos/zireael/nix-config/darwin/scripts/sync-ventoy.sh
```

Drop `nixos-25.11-x86_64-linux.iso` (graphical, ~3 GB) onto the Ventoy
partition. Ventoy boots vanilla NixOS ISOs as-is.

## BIOS settings (before the install boots)

Power on, mash F2 (or whatever Framework's boot prompt says) into BIOS:

1. **UMA Frame Buffer Size: 512 MB.** This is just the display framebuffer
   and is GPU-exclusive memory the OS can never reclaim — keep it tiny.
   The actual compute memory comes from GTT (next bullet), which is
   *shared* with the OS dynamically.
2. **Secure Boot: Off.** NixOS doesn't sign its bootloader by default;
   leaving Secure Boot on causes the install to fail at first reboot
   with `Verification failed: (0x1A) Security Violation`.
3. **TPM: On** (default). Not used yet, but harmless and may matter
   later if we add tpm2-based features.
4. **Boot order: Ventoy USB first.**

Save and exit. The kernel will then claim 124 GiB of system RAM as GTT
(amdgpu's translation table), addressable as VRAM by ROCm/Vulkan/HIP
workloads. See `nixos/mattfw/system.nix` `boot.kernelParams` for the
exact values (`amdgpu.gttsize=126976` + `ttm.pages_limit=32505856` —
both required, paired).

## Boot the installer

1. Insert Ventoy stick, power on, F12 for the boot menu, select Ventoy.
2. From Ventoy's menu, pick the NixOS 25.11 graphical ISO.
3. The live env boots into Plasma. Open Konsole — every command below
   runs from there.

### Get on the network

Wired ethernet: should work out of the box. Verify with `ping 1.1.1.1`.

WiFi (if needed): use the Plasma network applet in the system tray.

## Partition layout

Single 1 TB+ NVMe (whichever Framework SKU you got). No disk encryption
— the box lives in the home and trades the boot-time passphrase for
unattended reboots and SSH-only management.

Find the disk:

```bash
lsblk -d
# Look for the NVMe device. Typically /dev/nvme0n1 on Framework's M.2 slot.
```

Set a shell variable so the rest of the commands stay copy-pasteable:

```bash
DISK=/dev/nvme0n1
```

> **Confirm `$DISK` before you wipe it.** Anything below this line
> destroys the contents of `$DISK` without warning.

### Create partitions (GPT)

```bash
sudo parted "$DISK" -- mklabel gpt
sudo parted "$DISK" -- mkpart ESP fat32 1MiB 1025MiB           # 1 GiB EFI
sudo parted "$DISK" -- set 1 esp on
sudo parted "$DISK" -- mkpart swap linux-swap 1025MiB 33793MiB # 32 GiB swap
sudo parted "$DISK" -- mkpart root btrfs 33793MiB 100%         # rest = btrfs root
```

Verify:

```bash
sudo parted "$DISK" -- print
```

Partition device names depend on the disk type:

- NVMe: `${DISK}p1`, `${DISK}p2`, `${DISK}p3`
- SATA: `${DISK}1`, `${DISK}2`, `${DISK}3`

For NVMe (which the Framework Desktop uses):

```bash
ESP="${DISK}p1"
SWAP="${DISK}p2"
ROOT="${DISK}p3"
```

### Format

```bash
# EFI partition (label "BOOT" — matches fileSystems."/boot" in system.nix).
sudo mkfs.fat -F 32 -n BOOT "$ESP"

# btrfs root (label "nixos" — matches fileSystems entries in system.nix).
sudo mkfs.btrfs -f -L nixos "$ROOT"

# swap (label "swap" — matches swapDevices in system.nix).
sudo mkswap -L swap "$SWAP"
```

### Create btrfs subvolumes

Mount the raw btrfs volume, create subvolumes, unmount:

```bash
sudo mount "$ROOT" /mnt
sudo btrfs subvolume create /mnt/@
sudo btrfs subvolume create /mnt/@home
sudo btrfs subvolume create /mnt/@nix
sudo btrfs subvolume create /mnt/@log
sudo umount /mnt
```

### Mount everything for nixos-install

Mount in the order the installer expects: root first, then nested mounts.

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

## Run the installer

The flake is on the Ventoy stick (we synced it pre-install). Copy it
into the live env's tmpfs so `nixos-install` can read from a stable
path that survives the Ventoy unmount — `/tmp` here is the *live ISO's*
tmpfs, gone the moment we reboot. The real `~/repos/zireael/nix-config` on the
installed system gets created by the zireael checkout in the
post-install bootstrap, not by leftover staging files.

```bash
# Find the Ventoy mount. Live ISO usually has it under /run/media/<user>/Ventoy.
mkdir -p /tmp/flake
cp -r /run/media/*/Ventoy/home-manager/. /tmp/flake/

# Generate hardware-config that nixos-install merges with the flake.
# Our system.nix declares fileSystems explicitly, so the generated
# hardware-configuration.nix is mostly redundant — but `nixos-generate-config`
# also emits boot.initrd.availableKernelModules hints for the specific NVMe
# controller etc., which are worth a sanity check.
sudo nixos-generate-config --root /mnt --dir /tmp/nixos-gen
cat /tmp/nixos-gen/hardware-configuration.nix
# If anything in the generated initrd.availableKernelModules / boot.kernelModules
# isn't already implied by system.nix, copy those lines into system.nix.
# In practice for the Framework Desktop the auto-generated modules are
# already covered by linuxPackages_latest's defaults.

# Install.
sudo nixos-install --flake /tmp/flake#mattfw --no-root-passwd
```

`--no-root-passwd` is intentional — `nixos/common.nix` ships an
`initialHashedPassword` so root + mattw have a known login on first
boot, which `framework-bootstrap.sh` then rotates out of 1Password.

If the install completes without errors, reboot:

```bash
sudo reboot
```

Pull the Ventoy stick during the BIOS post.

## First boot

Login as `mattw` with the baked-in initial password from
`nixos/common.nix:51` (the bootstrap script rotates it immediately, so
this is a transient state).

After login, plug the Ventoy stick back in and run the bootstrap from
there — the dotfiles repo (which provides the real `~/repos/zireael/nix-config`)
hasn't been cloned yet, so we run from the stick for this one-time
bootstrap. After it completes, `~/repos/zireael/nix-config` exists and every future
`nix-switch` reads from there.

```bash
# Wait for Ventoy to mount. Path is one of:
#   /run/media/$USER/Ventoy/home-manager/...   (most live envs / SDDM auto-mount)
#   /run/media/mattw/Ventoy/home-manager/...   (logged-in user mount)
ls /run/media/*/Ventoy/home-manager/nixos/scripts/framework-bootstrap.sh

bash /run/media/*/Ventoy/home-manager/nixos/scripts/framework-bootstrap.sh
```

The bootstrap script handles:

1. Tailscale auth (interactive prompt for the pre-auth key, or pass
   `--auth-key tskey-...`).
2. Dotfiles clone (colocated jj + git at `$HOME`) — this is what creates the real
   `~/repos/zireael/nix-config` (it's a subdirectory inside the dotfiles tree, not
   a separate GitHub repo).
3. 1Password service-account token storage at
   `~/.config/op/service-account-token`.
4. `nixos-rebuild switch` against `~/repos/zireael/nix-config` (now that the user
   session + dotfiles + op token are in place, home-manager activations
   can fully run).
5. Password rotation from `op://Dev/Framework Password`.
6. Inter-server SSH key fetch.
7. Sanity checks (GPU detected, ROCm present, kernel params applied,
   GTT pinned).

## OpenClaw — first-time setup

mattfw is the primary OpenClaw host (replaced rpi5 in 2026-05). The
host-side module is `nixos/mattfw/openclaw.nix`; the gateway itself
is configured declaratively in `nixos/mattfw/home.nix` via the
`programs.openclaw` module from
[`nix-openclaw`](https://github.com/openclaw/nix-openclaw). The
gateway runs as a **systemd user service** (`openclaw-gateway.service`)
under mattw — no podman/container layer.

### 1Password prerequisites

Before the first `nix-switch mattfw` after a fresh install, the
following 1P items must exist — `openclaw-env-refresh.service` will
fail otherwise:

- `op://Server/OpenClaw Gateway Token/credential` — long random
  string used for `gateway.auth.token`. Rotate by regenerating +
  overwriting the 1P item.
- `op://Server/OpenClaw Discord Bot Token/credential` — Discord
  bot identity. Same bot can be reused if you're migrating from
  another host (one connection per token, so make sure the old
  host's service is stopped first).
- `op://Server/OpenClaw Workspace GH Token/token` — fine-grained
  PAT scoped to `mattwilkinsonn/openclaw-workspace`,
  `Contents: write`, no other permissions. Field name is `token`,
  not `credential` (1P's default field name varies by item template).
- `op://Server/Akiflow CLI Credentials` — 1P document item with a
  file attachment named `credentials.json`. Body is the contents
  of `~/.config/af/credentials.json` from the Mac after running
  `af auth`.
- `op://Server/OpenClaw OpenRouter API Key/credential`.
- `op://Server/OpenClaw Brave Search API Key/credential`.

### GitHub prerequisites

- Empty private repo `mattwilkinsonn/openclaw-workspace` exists with
  `main` as the default branch.
- The PAT above is generated and stored in 1P at the path above.

### How it boots

1. **`openclaw-env-refresh.service`** (root, oneshot) runs at boot,
   reads the 1P items above using mattw's service-account token at
   `~/.config/op/service-account-token`, and drops one file per
   secret into `/run/openclaw-secrets/`:
   - `gateway-token`, `discord-token`, `brave-search`, `openrouter`,
     `gh-workspace` — all mode `0600`, owned `mattw:users`.
2. **`openclaw-gateway.service`** (user service for mattw, provided
   by nix-openclaw) starts after the secrets exist. Config comes
   from `~/.openclaw/openclaw.json`, which Home Manager renders from
   `programs.openclaw.config` in `nixos/mattfw/home.nix`. Secrets
   are passed through the gateway wrapper's env-var substitution:
   when `programs.openclaw.environment.X` points at an existing file,
   the wrapper reads the file at startup and exports the variable
   from its contents — secrets never appear in a unit's `Environment=`
   line.
3. **`tailscale-serve-openclaw.service`** maps the tailnet:
   - `https://mattfw.tail08a5c5.ts.net/` → `:18789` (gateway + Control UI)
   - `https://mattfw.tail08a5c5.ts.net:9443/` → `:9090` (Cockpit)
4. **`openclaw-workspace-sync.timer`** (5 min after boot, then every
   10 min) snapshots `~/.openclaw/workspace/` to
   `mattwilkinsonn/openclaw-workspace` on GitHub. First sync no-ops
   if the workspace isn't a git repo yet (the gateway initializes
   it on first run).

### Verify after `nix-switch mattfw`

```bash
# 1. Secrets materialized?
sudo ls -la /run/openclaw-secrets/
# Expect: 5 files, mode 0600, owned mattw:users.

# 2. Gateway service up?
systemctl --user status openclaw-gateway
journalctl --user -u openclaw-gateway -n 50

# 3. Control UI reachable?
curl -fsS http://localhost:18789/health
# And the tailnet route:
#   https://mattfw.tail08a5c5.ts.net/

# 4. Workspace sync wiring sane?
systemctl list-timers openclaw-workspace-sync
journalctl -u openclaw-workspace-sync.service -n 30
```

If the gateway log shows `origin not allowed`, the Control UI's
`allowedOrigins` list in `programs.openclaw.config.gateway.controlUi`
needs another entry (then `nix-switch` + `systemctl --user restart
openclaw-gateway`). The defaults in `home.nix` cover the tailnet URL
and SSH-tunneled `http://localhost:18789`.

### Adding a Discord user / channel allowlist

The `channels.discord.allowFrom` list in `nixos/mattfw/home.nix` is
empty until you fill in your Discord user ID — until then the bot
ignores DMs/mentions. To find your ID, enable Developer Mode in
Discord, right-click your profile → Copy User ID.

### Adding more CLI tools to the gateway

`programs.openclaw.runtimePackages` in `nixos/mattfw/home.nix` is
the list of CLIs the gateway can shell out to. These land on the
gateway's PATH only, not mattw's login shell. Add by name from
nixpkgs:

```nix
runtimePackages = with pkgs; [
  git jq ripgrep curl gh chromium
  # add new tools here
];
```

### Akiflow CLI (`af`)

Installed into `~/.local/bin/af` by `shared/dev.nix`'s
`installAkiflowCli` activation. Credentials are dropped into
`~/.config/af/credentials.json` by `openclaw-env-refresh.service` on
every boot — no need to run `af auth` on mattfw (it would fail
anyway: no browser).

Test:

```bash
af auth status   # expect: authenticated
af ls            # expect: list of tasks
```

## Verify the GPU + memory layout

```bash
# Confirm the iGPU is detected as gfx1151 (Strix Halo).
rocminfo | grep -A1 'Marketing Name'

# Confirm both GTT params applied.
cat /proc/cmdline | tr ' ' '\n' | grep -E 'amdgpu|ttm'
# Expected: amdgpu.gttsize=126976  ttm.pages_limit=32505856

# Confirm the kernel actually pinned the full 124 GiB GTT range.
# This file reports bytes — divide by 1024^3 for GiB.
cat /sys/class/drm/card*/device/mem_info_gtt_total
# Expected: ~133143986176  (i.e. ~124 GiB)

# OS-visible RAM. With UMA=512MB carved out and GTT *shared*, MemTotal
# should read ~127 GiB (128 GiB physical minus the 512 MB framebuffer).
free -h
```

If `MemTotal` shows materially less than 127 GiB, your BIOS UMA Frame
Buffer is set higher than 512 MB — drop it and reboot. UMA is GPU-
exclusive RAM that the OS can never see; GTT is *shared* RAM that the
GPU can pin on demand. We want as much of the latter and as little of
the former as possible.

## GTT memory reference

If 124 GiB is too tight for OS workloads, reduce both knobs together
in `system.nix`. The two values are paired (`pages_limit × 4 KiB =
gttsize × 1 MiB`):

| `amdgpu.gttsize` (MiB) | `ttm.pages_limit` | GTT (GiB) | OS headroom (GiB) |
| ---------------------- | ----------------- | --------- | ----------------- |
| 126976 | 32505856 | 124 | ~4 |
| 122880 | 31457280 | 120 | ~8 |
| 117760 | 30146560 | 115 | ~13 |
| 112640 | 28835840 | 110 | ~18 |

## SSH from the Mac

Add to `~/.ssh/config` (Mac side):

```ssh-config
Host mattfw
  HostName mattfw.tail08a5c5.ts.net
  User mattw
  IdentityAgent ~/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock
```

Plain `ssh mattfw` works for shell access, log inspection, and running
`distrobox enter llama-rocm` to drive the inference container. VSCode
Remote-SSH also works against this host if you need to peek at config
or model files in an editor — it's just not the primary dev box (use
the mattpc-wsl machine for that).

## Local Plasma session (debug only)

```bash
sudo systemctl start sddm                 # one-off
# or
sudo systemctl isolate graphical.target   # this boot only
sudo systemctl isolate multi-user.target  # back to headless
```

Default boot target is `multi-user.target` — every reboot returns to
headless TTY automatically.

## Local LLM stack

The Strix Halo community has converged on **kyuz0/amd-strix-halo-toolboxes**
(<https://strix-halo-toolboxes.com>) — pre-built containers with ROCm +
patched llama.cpp + rocWMMA, rebuilt automatically against llama.cpp
master. Two backends worth keeping side-by-side:

- `rocm-7.2.2` — best performance, recommended default for everything.
- `vulkan-radv` — most stable / compatible if a model misbehaves on ROCm.

> Avoid `rocm7-nightlies` for now — there's an open bug capping
> allocation at 64 GB which defeats the entire point of this box.
> Track <https://github.com/ROCm/TheRock/issues/4645>.

### Set up distrobox + the containers

`distrobox` and `podman` are already in the system from
`virtualisation.podman.enable = true` in `nixos/common.nix`. Pull the
containers:

```bash
nix shell nixpkgs#distrobox

# ROCm 7.2.2 (primary).
distrobox create llama-rocm \
  --image docker.io/kyuz0/amd-strix-halo-toolboxes:rocm-7.2.2 \
  --additional-flags "--device /dev/dri --device /dev/kfd --group-add video --group-add render --security-opt seccomp=unconfined"

# Vulkan RADV (fallback for compatibility).
distrobox create llama-vulkan \
  --image docker.io/kyuz0/amd-strix-halo-toolboxes:vulkan-radv \
  --additional-flags "--device /dev/dri --group-add video --security-opt seccomp=unconfined"
```

Enter and verify:

```bash
distrobox enter llama-rocm
llama-cli --list-devices
# Should list a gfx1151 device with ~124 GiB available.
```

### Required inference flags on Strix Halo

Both upstream guides (`Framework-strix-halo-llm-setup` and
`amd-strix-halo-toolboxes`) agree on these — they're not optional:

- `-fa 1` — flash attention. Required, avoids crashes/slowdowns.
- `--no-mmap` — required for GPU backends. Without this, model loads
  are extremely slow and memory fragments.
- `-ngl 999` — offload all layers to GPU.
- `llama-bench` uses `-mmp 0` instead of `--no-mmap`.

Server example:

```bash
llama-server \
  -m models/some-model.gguf \
  -c 8192 -ngl 999 -fa 1 --no-mmap \
  --host 0.0.0.0 --port 8080
```

### Building llama.cpp directly (alternative to the container)

```bash
nix shell nixpkgs#cmake nixpkgs#gcc

git clone https://github.com/ggerganov/llama.cpp.git
cd llama.cpp
cmake -B build -S . -DGGML_HIP=ON -DAMDGPU_TARGETS=gfx1151
cmake --build build --config Release -j$(nproc)
```

### VRAM estimation

The kyuz0 repo ships `gguf-vram-estimator.py` which reads a `.gguf`
header and projects total VRAM (model + context overhead) at any
context length. Use it before downloading enormous models — at 124 GiB
of GTT, 70B Q4 is comfortable, 235B Q3 fits up to ~130k context, etc.
See <https://github.com/kyuz0/amd-strix-halo-toolboxes/blob/main/docs/vram-estimator.md>.

### Firmware regression to know about

`linux-firmware-20251125` broke ROCm on gfx1151 (instability / crashes).
`20251111` is the last good version. NixOS pulls firmware from the
`linux-firmware` package in nixpkgs — if ROCm starts misbehaving after
a `nix flake update`, this is the first thing to check. See
<https://github.com/kyuz0/amd-strix-halo-toolboxes/blob/main/docs/troubleshooting-firmware.md>.
