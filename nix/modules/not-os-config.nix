{ lib, pkgs, ... }:
{
  environment.systemPackages = [
    pkgs.utillinux
    pkgs.gnugrep
    pkgs.kmod
    pkgs.devmem2
    # for debugging
    pkgs.strace
  ];

  environment.pathsToLink = [ "/lib/modules" ];

  networking.timeServers = [];

  not-os.nix = true;
  not-os.simpleStaticIp = true;
  not-os.preMount = ''
    echo 'nixos' > /proc/sys/kernel/hostname
    ip addr add 127.0.0.1/8 dev lo
    ip addr add ::1/128 dev lo
    ip link set dev lo up
    ip addr add 10.0.2.15/24 dev eth0
  '';

  boot.initrd.availableKernelModules = [ "virtio_console" "virtio_mmio" "9p" "9pnet_virtio" ];
  boot.initrd.kernelModules = [ "virtio_mmio" "9p" "9pnet_virtio" ];

  system.activationScripts.vmsh = ''
    mkdir /vmsh
    mount -t 9p vmsh /vmsh -o "trans=virtio";
  '';

  environment.etc = {
    "hosts".text = ''
      127.0.0.1 localhost
      ::1 localhost
      127.0.0.1 nixos
      ::1 nixos
    '';
    "ssh/authorized_keys.d/root" = {
      source = ../ssh_key.pub;
      mode = "444";
    };
    "service/shell/run".source = pkgs.writeScript "shell" ''
      #!/bin/sh
      export USER=root
      export HOME=/root
      cd $HOME

      source /etc/profile

      exec < /dev/ttyS0 > /dev/ttyS0 2>&1
      echo "If you are connect via serial console:"
      echo "Type Ctrl-a c to switch to the qemu console"
      echo "and 'quit' to stop the VM."
      exec ${pkgs.utillinux}/bin/setsid ${pkgs.bash}/bin/bash -l
    '';
  };
  environment.etc.profile.text = ''
    export PS1="\e[0;32m[\u@\h \w]\$ \e[0m"
  '';
}
