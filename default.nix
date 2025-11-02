{
  pkgs ? import <nixpkgs> { },
  lib,
}:
pkgs.rustPlatform.buildRustPackage rec {
  nativeBuildInputs = [
    pkgs.pkg-config
    pkgs.wrapGAppsHook3
  ];
  buildInputs = [
    pkgs.vulkan-loader
    pkgs.gtk4
    pkgs.gtk4-layer-shell
    pkgs.glib
    pkgs.gst_all_1.gstreamer
    pkgs.gst_all_1.gst-plugins-base
    pkgs.gst_all_1.gst-plugins-good
    pkgs.gst_all_1.gst-plugins-bad
    pkgs.gst_all_1.gst-plugins-ugly
    pkgs.gst_all_1.gst-libav
  ];
  pname = "clips-workspace";
  version = "1.0";
  cargoLock.lockFile = ./Cargo.lock;
  src = pkgs.lib.cleanSource ./.;
  cargoHash = lib.fakeHash;
}
