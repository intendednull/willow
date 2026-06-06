{
  # willow — P2P Discord replacement, deployed via the shared `infra` NixOS flake.
  #
  # This is a `runtime = "multi"` app (../infra ONBOARDING.md §2b): one repo, four
  # cooperating units (web UI + relay + replay + storage worker). The three infra
  # contract outputs:
  #   packages.${system}.{relay,replay,storage,web,default} — the build artifacts
  #   nixosModules.default = import ./nix/module.nix self    — the multi-unit service
  #     module (closes over `self` so it can reach self.packages.<system>.*)
  # A multi app does NOT need packages.oci (mkApp's multi branch never images it —
  # moonlitmorphs omits it too).
  description = "willow — P2P Discord replacement (relay + workers + web), deployed via infra";

  inputs = {
    # Pin nixpkgs to infra's release so the closure dedups to one nixpkgs.
    # (infra re-points this input's nixpkgs to its own via `follows`.)
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    crane.url = "github:ipetkov/crane";
    crane.inputs.nixpkgs.follows = "nixpkgs";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = inputs@{ self, nixpkgs, crane, rust-overlay, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      };
      lib = pkgs.lib;

      # Toolchain from rust-toolchain.toml — already includes the wasm32 target +
      # stable channel, so the same toolchain builds the native servers AND the
      # wasm web app. crane's overrideToolchain takes a FUNCTION of pkgs.
      craneLib = (crane.mkLib pkgs).overrideToolchain
        (p: p.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml);

      # ---- sources -------------------------------------------------------------
      # Servers: crane's default filter (Rust/Cargo only) is enough.
      serverSrc = craneLib.cleanCargoSource ./.;
      # Web: trunk needs the non-Rust assets (css/js/svg/manifest/webm/index.html/
      # Trunk.toml) that crane's filter would strip. Union the whole crates/web dir
      # (which also re-includes its .rs — fileset unions dedupe) with the Cargo sources.
      webSrc = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          (craneLib.fileset.commonCargoSources ./.)
          ./crates/web
        ];
      };

      # ---- native server binaries ---------------------------------------------
      mkBin = crate: craneLib.buildPackage {
        pname = crate;
        version = "0.1.0";
        src = serverSrc;
        cargoExtraArgs = "-p ${crate}";
        # The deploy build is not a test runner (infra ONBOARDING gotcha): tests run
        # in willow's own CI; the /healthz gate covers the running service.
        doCheck = false;
      };
      relay = mkBin "willow-relay";
      replay = mkBin "willow-replay";
      storage = mkBin "willow-storage";

      # ---- wasm web app (trunk) ------------------------------------------------
      # wasm-bindgen-cli MUST match the wasm-bindgen crate version locked in
      # Cargo.lock (0.2.118) or trunk's wasm-bindgen step errors on a version skew.
      # nixpkgs nixos-26.05 ships 0.2.121, so pin 0.2.118 via the buildWasmBindgenCli
      # helper with our own fetched src + vendored deps. (override only exposes
      # buildWasmBindgenCli/fetchCrate/rustPlatform, not version — so build it directly.)
      wbgVersion = "0.2.118";
      wbgSrc = pkgs.fetchCrate {
        pname = "wasm-bindgen-cli";
        version = wbgVersion;
        hash = "sha256-ve783oYH0TGv8Z8lIPdGjItzeLDQLOT5uv/jbFOlZpI=";
      };
      wasmBindgenCli = pkgs.buildWasmBindgenCli {
        version = wbgVersion;
        src = wbgSrc;
        cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
          src = wbgSrc;
          name = "wasm-bindgen-cli-${wbgVersion}-vendor";
          hash = "sha256-EYDfuBlH3zmTxACBL+sjicRna84CvoesKSQVcYiG9P0=";
        };
      };

      # `ring` (via iroh-relay → rustls) compiles a little C for its bignum routines.
      # On wasm32-unknown-unknown its build.rs needs a clang that can emit wasm objects;
      # the stdenv default cc is gcc (host-only), so without this ring silently skips the
      # C and the wasm link fails with `undefined symbol: ring_core_*__limbs_mul_add_limb`.
      # Point ring's `cc`/`ar` at the unwrapped LLVM tools for the wasm target.
      webEnv = {
        CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
        CC_wasm32_unknown_unknown = "${pkgs.llvmPackages.clang-unwrapped}/bin/clang";
        AR_wasm32_unknown_unknown = "${pkgs.llvmPackages.bintools-unwrapped}/bin/llvm-ar";
      };

      # wasm deps, prebuilt for the wasm32 target so the trunk build is incremental.
      webCargoArtifacts = craneLib.buildDepsOnly (webEnv // {
        pname = "willow-web-deps";
        version = "0.1.0";
        src = webSrc;
        cargoExtraArgs = "-p willow-web";
        doCheck = false;
      });

      web = craneLib.buildTrunkPackage (webEnv // {
        pname = "willow-web";
        version = "0.1.0";
        src = webSrc;
        cargoArtifacts = webCargoArtifacts;
        wasm-bindgen-cli = wasmBindgenCli;
        # trunk's wasm-opt step (index.html: data-wasm-opt="z").
        nativeBuildInputs = [ pkgs.binaryen ];
        # crane runs `trunk build` from the workspace ROOT, where trunk resolves the
        # Cargo project from its cwd → the virtual root manifest → "could not find the
        # root package of the target crate". Run trunk from inside crates/web (a subshell
        # so the install step still copies from the workspace root). cargo still finds the
        # workspace root + crane's prebuilt target/ + CARGO_HOME vendor config (all absolute).
        #
        # --no-sri: the edge serves these assets compressed (static-web-server zstd), and
        # Chrome's SRI check rejects the integrity-tagged module/wasm under that encoding,
        # so the WASM never instantiates → blank "Loading…" page. SRI adds ~nothing for
        # same-origin first-party assets (the same origin serves index.html AND the hashes;
        # TLS already covers MITM), so drop it for the deploy build. Local `trunk serve`
        # (uncompressed) keeps SRI — this only affects the compressed production artifact.
        buildPhaseCargoCommand = ''
          ( cd crates/web && trunk build --release=true --no-sri=true index.html )
        '';
        installPhaseCommand = ''
          cp -r crates/web/dist $out
        '';
      });
    in
    {
      packages.${system} = {
        inherit relay replay storage web;
        inherit wasmBindgenCli; # TEMP: isolate to harvest override hashes; removed before finalize
        # Contract wants packages.default; a multi app doesn't use it at deploy time,
        # so join the artifacts for a meaningful `nix build`.
        default = pkgs.symlinkJoin {
          name = "willow-all";
          paths = [ relay replay storage web ];
        };
      };

      # Multi-unit service module, closing over `self` for self.packages.<system>.*.
      nixosModules.default = import ./nix/module.nix self;

      devShells.${system}.default = pkgs.mkShell {
        inputsFrom = [ relay ];
        packages = [ pkgs.trunk wasmBindgenCli pkgs.binaryen ];
      };
    };
}
