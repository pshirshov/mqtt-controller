{
  description = "Unified zigbee2mqtt provisioner and runtime controller";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    crane.url = "github:ipetkov/crane";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
      rust-overlay,
      ...
    }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      packagesFor =
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
          pkgsBuild = import nixpkgs { system = "x86_64-linux"; };
          frontend = pkgsBuild.callPackage ./nix/frontend.nix { };
          source = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter =
              name: type:
              let
                baseName = baseNameOf (toString name);
              in
              !(
                type == "directory"
                && builtins.elem baseName [
                  ".git"
                  "frontend"
                  "target"
                ]
              );
          };
          craneLib = (crane.mkLib pkgs).overrideToolchain (p: p.rust-bin.stable.latest.minimal);
          commonArgs = {
            src = source;
            cargoToml = "${source}/crates/mqtt-controller/Cargo.toml";
            cargoExtraArgs = "-p mqtt-controller";
            nativeBuildInputs = [ pkgs.mold ];
          };
          cargoArtifacts = craneLib.buildDepsOnly (
            commonArgs
            // {
              doCheck = false;
            }
          );
          controller = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoTestExtraArgs = "-p mqtt-controller -p mqtt-controller-wire";
              doCheck = true;
              postInstall = ''
                mkdir -p $out/share/mqtt-controller
                cp -r ${frontend} $out/share/mqtt-controller/web
                chmod -R u+w $out/share/mqtt-controller/web
              '';
              passthru.mqtt-controller-frontend = frontend;
              meta = with pkgs.lib; {
                description = "Unified zigbee2mqtt provisioner and runtime controller";
                license = licenses.mit;
                maintainers = with maintainers; [ pshirshov ];
                mainProgram = "mqtt-controller";
                platforms = platforms.linux;
              };
            }
          );
        in
        {
          default = controller;
          mqtt-controller = controller;
          mqtt-controller-frontend = frontend;
        };
    in
    {
      packages = forAllSystems packagesFor;

      overlays.default = final: _previous: {
        inherit (self.packages.${final.stdenv.hostPlatform.system})
          mqtt-controller
          mqtt-controller-frontend
          ;
      };

      nixosModules.default = {
        imports = [ ./nix/module.nix ];
        nixpkgs.overlays = [ self.overlays.default ];
      };
    };
}
