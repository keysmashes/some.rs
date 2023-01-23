{
  description = "a meta-pager";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
  };

  outputs = inputs: let
    systems = [ "x86_64-linux" "x86_64-darwin" "aarch64-linux" "aarch64-darwin" ];
    someFor = system: let
      pkgs = import inputs.nixpkgs { inherit system; };
    in someWith pkgs;
    someWith = pkgs: pkgs.rustPlatform.buildRustPackage {
      pname = "some";
      version = "0.1.0";
      src = ./.;
      cargoSha256 = "sha256-U6fqjOy8GW0hFsoyHz1f2iYRbbsxvLEKDB+rNNKwBhw=";
      meta = {
        description = "a meta-pager";
        homepage = "https://github.com/sersorrel/some.rs";
        mainProgram = "some";
      };
    };
  in {
    packages = builtins.listToAttrs (map (system: { name = system; value = { default = someFor system; }; }) systems);
    apps = builtins.listToAttrs (map (system: { name = system; value = { default = { type = "app"; program = "${someFor system}/bin/some"; }; }; }) systems);
    overlays.default = final: prev: someWith final;
  };
}
