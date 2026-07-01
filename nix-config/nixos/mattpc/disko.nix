_:
# Declarative disk layout for mattpc (bare-metal). disko owns Disk 0 ONLY —
# the 2 TB Samsung 980 PRO NVMe that previously held the WSL vhdx. Windows
# lives on Disk 1 and is NEVER referenced here, so `disko` cannot touch it.
#
# ── SAFETY: the two NVMe drives are the IDENTICAL model, so the device MUST
# be pinned by-id (serial), never by /dev/nvmeXn1 (enumeration order isn't
# stable and could point at the Windows disk). Fill `device` below at install
# time from the installer:
#
#     ls -l /dev/disk/by-id/ | grep -i nvme
#
# Pick the by-id whose serial is Disk 0's — the WSL/"Games" disk, NOT
# Windows. Windows (Disk 1) reported serial 0025_38B2_4140_9DA2; the target
# (Disk 0) reported 0025_38BA_21A1_303A (Windows-formatted — the Linux by-id
# serial differs, so identify by ELIMINATION: the NVMe that does NOT contain
# the ~200 MB vfat ESP + the large NTFS "Root"/Windows partition is Disk 0).
# Confirm with `lsblk -f` before running disko: the target disk should show a
# single large NTFS ("Games") and nothing else.
#
# Layout (per the approved plan): 1 GiB ESP (vfat) + 64 GiB swap (= RAM, for
# hibernate) + btrfs root with @ / @home / @nix subvolumes (zstd-compressed).
{
  disko.devices = {
    disk = {
      main = {
        type = "disk";
        # REPLACE at install with Disk 0's by-id path, e.g.
        # /dev/disk/by-id/nvme-Samsung_SSD_980_PRO_2TB_<serial>
        device = "/dev/disk/by-id/REPLACE-WITH-DISK0-BY-ID";
        content = {
          type = "gpt";
          partitions = {
            ESP = {
              priority = 1;
              name = "ESP";
              size = "1G";
              type = "EF00";
              content = {
                type = "filesystem";
                format = "vfat";
                mountpoint = "/boot";
                mountOptions = [ "umask=0077" ];
              };
            };
            swap = {
              priority = 2;
              name = "swap";
              size = "64G";
              content = {
                type = "swap";
                # Resume device for hibernate (swap = RAM size).
                resumeDevice = true;
              };
            };
            root = {
              priority = 3;
              name = "root";
              size = "100%";
              content = {
                type = "btrfs";
                extraArgs = [ "-f" ];
                subvolumes = {
                  "@" = {
                    mountpoint = "/";
                    mountOptions = [
                      "compress=zstd"
                      "noatime"
                    ];
                  };
                  "@home" = {
                    mountpoint = "/home";
                    mountOptions = [
                      "compress=zstd"
                      "noatime"
                    ];
                  };
                  "@nix" = {
                    mountpoint = "/nix";
                    mountOptions = [
                      "compress=zstd"
                      "noatime"
                    ];
                  };
                };
              };
            };
          };
        };
      };
    };
  };
}
