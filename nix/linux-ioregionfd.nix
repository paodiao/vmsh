{ buildLinux, fetchFromGitHub, linuxPackages_5_15, fetchurl, modDirVersionArg ? null, ... }@args:

buildLinux (args // rec {
  version = "5.14.16";
  modDirVersion = if (modDirVersionArg == null) then
    builtins.replaceStrings [ "-" ] [ ".0-" ] version
      else
    modDirVersionArg;
  src = fetchFromGitHub {
    owner = "Mic92";
    repo = "linux";
    rev = "837381c3b1499bfb5aa7040b05ae01f2b4c1c758";
    sha256 = "sha256-Ozp0D2dpn9rIiwme0YZeM4S02jumsSY6ToHiXU1loww=";
  };

  kernelPatches = [{
    name = "enable-kvm-ioregion";
    patch = null;
    # we need XFS_ONLINE_SCRUB this for xfstests
    extraConfig = ''
      KVM_IOREGION y
      XFS_ONLINE_SCRUB y
    '';
  # 5.12 patch list has one fix we already have in our branch
  }] ++ linuxPackages_5_15.kernel.kernelPatches;
  extraMeta.branch = "5.14";
  ignoreConfigErrors = true;
} // (args.argsOverride or { }))
