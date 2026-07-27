let
  nixpkgs = builtins.fetchTarball {
    url = "https://channels.nixos.org/nixos-26.05/nixexprs.tar.xz";
  };

  pkgs = import nixpkgs {};
in

pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    rustc
    cargo
    pkg-config
  ];

  buildInputs = with pkgs; [
    libadwaita
  ];
}

