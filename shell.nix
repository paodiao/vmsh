{ pkgs ? (import (import ./nix/sources.nix).nixpkgs { }) }:

let
  sources = import ./nix/sources.nix;
  naersk = pkgs.callPackage sources.naersk { };
  niv = pkgs.callPackage sources.niv { };

  vmsh = pkgs.callPackage ./vmsh.nix {
    inherit naersk;
  };
in
pkgs.mkShell {
  RUST_SRC_PATH = pkgs.rustPlatform.rustLibSrc;
  nativeBuildInputs = [
    niv.niv
    pkgs.rust-analyzer
    pkgs.rustfmt
    pkgs.just
    pkgs.qemu_kvm
    pkgs.clippy
    pkgs.rustfmt
    pkgs.rustc
    pkgs.cargo-watch
    pkgs.cargo-deny
    pkgs.pre-commit
    pkgs.git # needed for pre-commit install
    (pkgs.python3.withPackages(ps: [ps.pytest ps.black ps.flake8 ps.mypy]))
    pkgs.tmux # needed for integration test
  ] ++ vmsh.nativeBuildInputs;
  buildInputs = vmsh.buildInputs;
  shellHook = ''
    pre-commit install
  '';
}
