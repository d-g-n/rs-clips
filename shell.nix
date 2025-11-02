{
  pkgs ? import <nixpkgs> { },
}:
pkgs.mkShell {
  # Get dependencies from the main package
  inputsFrom = [ (pkgs.callPackage ./default.nix { }) ];
  # Additional tooling
  nativeBuildInputs = with pkgs; [
    cargo
    rustc
    rust-analyzer
    pkg-config
    wrapGAppsHook
    clippy
    rustfmt
  ];
  buildInputs = with pkgs; [

    gtk4
    gtk4-layer-shell
    glib
    pango
    cairo
    gdk-pixbuf
    graphene
    atk
    freetype
    fontconfig
    harfbuzz
  ];
  RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
}
