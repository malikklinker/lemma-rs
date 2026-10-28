{
  description = "lemma-rs Rust / TSX motion graphics development";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in {
      devShells = forAllSystems (system:
        let pkgs = import nixpkgs { inherit system; };
        in {
          default = pkgs.mkShell {
            packages = with pkgs; [ cargo rustc rustfmt clippy nodejs pkg-config ffmpeg-headless ];
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [
              vulkan-loader wayland libxkbcommon libGL
              libx11 libxcursor libxrandr libxi
            ]);
            shellHook = ''
              export LD_LIBRARY_PATH="/run/opengl-driver/lib:$LD_LIBRARY_PATH"
            '';
          };
        });
    };
}
