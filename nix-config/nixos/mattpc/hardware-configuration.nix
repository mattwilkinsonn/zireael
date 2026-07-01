{ lib, modulesPath, ... }:
# Bare-metal hardware profile for mattpc (i9-14900KS + RTX 4080 + NVMe).
#
# disko (disko.nix) generates all `fileSystems`/`swapDevices` entries, so this
# file deliberately declares NONE — it carries only the kernel/initrd modules
# and CPU microcode a normal `nixos-generate-config` would emit.
#
# AT INSTALL: run `nixos-generate-config --no-filesystems --root /mnt` and
# reconcile the generated `boot.initrd.availableKernelModules` with the list
# below (hardware can surface a module this generic set misses). `--no-filesystems`
# keeps disko as the single source of the mount layout.
{
  imports = [ (modulesPath + "/installer/scan/not-detected.nix") ];

  # Typical initrd modules for a modern Intel + NVMe + USB desktop. The
  # installer's scan is authoritative; merge anything extra it finds.
  boot.initrd.availableKernelModules = [
    "xhci_pci"
    "ahci"
    "nvme"
    "usbhid"
    "usb_storage"
    "sd_mod"
    "thunderbolt"
  ];
  boot.initrd.kernelModules = [ ];
  boot.kernelModules = [ "kvm-intel" ];
  boot.extraModulePackages = [ ];

  # 14th-gen Intel microcode.
  hardware.cpu.intel.updateMicrocode = lib.mkDefault true;
  hardware.enableRedistributableFirmware = lib.mkDefault true;

  nixpkgs.hostPlatform = lib.mkDefault "x86_64-linux";
}
