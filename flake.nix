{
  description = "Local-first scientific CAD worker, isolated adapters and explicit qualification";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/73e728ddb6b7a12d18808f510813a13ee1fe4cce";
    harbor-rs = {
      url = "github:caniko/harbor-rs/c4ffa5d9b9232eae1f6693dfbcbcdd2548f31592";
      flake = false;
    };
    harbor-py = {
      url = "github:caniko/harbor-py/dab2acaf106eab7f94084ed8d02747c495f31737";
      flake = false;
    };
    fleetix = {
      url = "github:caniko/fleetix/2230d9ee804a66d94424a91919182e4fcca13ab2";
      flake = false;
    };
    openlb = {
      url = "git+https://gitlab.com/openlb/release.git?ref=1.9.0&rev=145cd54810b468f4b6fd3ed86b10644264841578";
      flake = false;
    };
    crane.url = "github:ipetkov/crane/73b980519cefc727a5f6cc8e5c0947a2f9be6edd";
    rust-overlay = {
      url = "github:oxalica/rust-overlay/89e99bf0778a8f2cd18c9360c3f19c1ee47fc739";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    pyproject-nix = {
      url = "github:pyproject-nix/pyproject.nix/7af23cfe91064865ecf2e835da28b45b3c6f49fd";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    uv2nix = {
      url = "github:pyproject-nix/uv2nix/2f698e4b5a3c6004edaf051543542f18a36afe77";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.pyproject-nix.follows = "pyproject-nix";
    };
    pyproject-build-systems = {
      url = "github:pyproject-nix/build-system-pkgs/430680a19bc85a3bda55f12e4cc1a1aadcf2e478";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.pyproject-nix.follows = "pyproject-nix";
      inputs.uv2nix.follows = "uv2nix";
    };
    treefmt-nix = {
      url = "github:numtide/treefmt-nix/27b3b12a8e6375f28ebe122f07d230ca5459bbfa";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = inputs @ {
    self,
    nixpkgs,
    fleetix,
    ...
  }: let
    system = "x86_64-linux";
    # Import the pinned reusable APIs directly; unrelated upstream outputs remain lazy.
    rust = import (inputs.harbor-rs + "/lib") {
      inherit (inputs) crane;
      osxcross = {};
    };
    python = import (inputs.harbor-py + "/lib") {
      inherit nixpkgs;
      inherit (inputs) pyproject-nix uv2nix pyproject-build-systems;
    };
    gpuLib = import (fleetix + "/lib/gpu.nix");
    pkgs = python.mkPkgs {
      inherit system;
      overlays = [inputs.rust-overlay.overlays.default];
      config.allowUnfree = false;
    };
    # A separate package set, never a broad unfree default or host driver package.
    cudaPkgs = python.mkPkgs {
      inherit system;
      config = {
        allowUnfree = false;
        allowUnfreePredicate = p:
          builtins.elem (pkgs.lib.getName p) [
            "cuda_cccl"
            "cuda_cudart"
            "cuda_nvcc"
            "cuda_cupti"
            "cuda_nvrtc"
            "cuda_cuobjdump"
            "cuda_nvdisasm"
            "cuda_gdb"
            "cuda_nvml_dev"
            "cuda_profiler_api"
          ];
        cudaCapabilities = ["8.6"];
      };
    };
    toolchain = rust.mkToolchain {
      inherit pkgs;
      toolchainFile = ./rust-toolchain.toml;
      withRustAnalyzer = false;
      cache.enable = false;
    };
    inherit (toolchain) craneLib;
    source = pkgs.lib.cleanSourceWith {
      src = ../..;
      filter = path: type:
        craneLib.filterCargoSources path type
        || pkgs.lib.hasInfix "/profiles/" path
        || pkgs.lib.hasSuffix "/examples/thermal-contact.json" path
        || pkgs.lib.hasSuffix "/examples/freezing-reference.json" path
        || pkgs.lib.hasSuffix "/examples/snow-reference.json" path
        || pkgs.lib.hasSuffix "/examples/spectral-reference.json" path
        || pkgs.lib.hasSuffix "/examples/atmosphere-reference.json" path
        || pkgs.lib.hasSuffix "/examples/atmosphere-transfer.json" path
        || pkgs.lib.hasSuffix "/examples/cad-spectral-scene.json" path
        || pkgs.lib.hasSuffix "/examples/cad-spectral-transport.json" path
        || pkgs.lib.hasSuffix "/examples/retained-cooling.json" path
        || pkgs.lib.hasSuffix "/adapters/openlb_retained_cooling.cpp" path
        || pkgs.lib.hasSuffix "/nix/patches/calculix-temperature-precision.patch" path;
    };
    common = {
      src = source;
      cargoExtraArgs = "--locked -p harbor-cad";
      strictDeps = true;
      pname = "harbor-cad";
      version = "0.1.0";
      CARGO_BUILD_JOBS = "2";
      HARBOR_CAD_SYSTEMCTL = "${pkgs.systemd}/bin/systemctl";
      HARBOR_CAD_SYSTEMD_RUN = "${pkgs.systemd}/bin/systemd-run";
      HARBOR_CAD_NIX_STORE = "${pkgs.nix}/bin/nix-store";
    };
    cargoArtifacts = craneLib.buildDepsOnly common;
    cli = craneLib.buildPackage (common // {inherit cargoArtifacts;});
    mcpEnv = python.mkUvVirtualEnv {
      inherit pkgs;
      python = pkgs.python313;
      name = "harbor-cad-mcp-env";
      workspaceRoot = ./python;
      dependencies = {harbor-cad-mcp = [];};
    };
    mcp = python.mkPythonApplicationPackage {
      inherit pkgs;
      name = "harbor-cad-mcp";
      environment = mcpEnv;
      scripts = ["harbor-cad-mcp"];
      includeEnvironment = false;
    };
    testEnv = python.mkUvVirtualEnv {
      inherit pkgs;
      python = pkgs.python313;
      name = "harbor-cad-tests";
      workspaceRoot = ./python;
      dependencies = (python.loadUvWorkspace {workspaceRoot = ./python;}).deps.all;
    };
    native = import ./nix/native.nix {inherit pkgs cudaPkgs inputs;};
    filters = import ./nix/filters.nix {inherit pkgs;};
    fem = import ./nix/fem.nix {inherit pkgs;};
    thermal = import ./nix/thermal.nix {inherit pkgs fem;};
    cadMesh = import ./nix/cad-mesh.nix {inherit pkgs fem;};
    femImported = import ./nix/fem-imported.nix {inherit pkgs fem cadMesh;};
    wetting = import ./nix/wetting.nix {inherit pkgs inputs cudaPkgs;};
    contact = import ./nix/contact.nix {inherit pkgs fem;};
    freezing = import ./nix/freezing.nix {inherit pkgs inputs cudaPkgs;};
    retainedCooling = import ./nix/retained-cooling.nix {inherit pkgs inputs cudaPkgs;};
    spectral = import ./nix/spectral.nix {inherit pkgs;};
    atmosphere = import ./nix/atmosphere.nix {inherit pkgs;};
    format = inputs.treefmt-nix.lib.evalModule pkgs {
      projectRootFile = "flake.nix";
      programs.rustfmt = {
        enable = true;
        package = toolchain.rustToolchain;
        edition = "2024";
      };
      programs.alejandra.enable = true;
      programs.ruff-format.enable = true;
    };
  in {
    # Canix's scoped updater reads this ownership contract before any lock
    # mutation. All inputs here are explicit immutable pins, not cache-managed.
    cachePinMeta = {
      schemaVersion = 4;
      pins = {};
    };
    lib = {
      inherit gpuLib;
      fleetixRevision = fleetix.rev;
      fleetixContractDigest = builtins.hashString "sha256" (builtins.readFile (fleetix + "/lib/generated/gpu-contract.json"));
    };
    packages.${system} = {
      default = cli;
      worker = cli;
      inherit mcp cargoArtifacts;
      inherit (native) cad visualization media runtime-cpu openlb-cpu openlb-cuda openlb-hip runtime-cuda runtime-hip;
      inherit (filters) kokkos-hip vtk-hip filter-hip;
      inherit (fem) fem-cpu runtime-fem-cpu runtime-fem-worker;
      inherit (thermal) thermal-cpu runtime-thermal-cpu runtime-thermal-worker;
      inherit (cadMesh) cad-mesh-cpu runtime-cad-mesh-cpu runtime-cad-mesh-worker;
      inherit (femImported) fem-imported-cpu runtime-fem-imported-cpu runtime-fem-imported-worker;
      inherit (wetting) wetting-reference-cpu runtime-wetting-reference-cpu runtime-wetting-worker;
      inherit (contact) contact-reference-cpu runtime-contact-reference-cpu runtime-contact-worker;
      inherit (freezing) freezing-native-cpu freezing-reference-cpu runtime-freezing-reference-cpu runtime-freezing-worker;
      inherit (retainedCooling) retained-cooling-native-cpu retained-cooling-reference-cpu runtime-retained-cooling-reference-cpu runtime-retained-cooling-worker;
      inherit (spectral) spectral-environment-cpu spectral-mitsuba spectral-drjit spectral-reference-cpu runtime-spectral-reference-cpu runtime-spectral-worker;
      inherit (spectral) atmospheric-spectral-reference-cpu runtime-atmospheric-spectral-reference-cpu runtime-atmospheric-spectral-worker;
      inherit (spectral) cad-spectral-direct-reference-cpu runtime-cad-spectral-direct-reference-cpu runtime-cad-spectral-direct-worker;
      inherit (atmosphere) atmosphere-native-cpu atmosphere-reference-cpu runtime-atmosphere-reference-cpu runtime-atmosphere-worker;
      runtime-thermal-contact-worker = pkgs.writeText "harbor-cad-native-runtime.json" (builtins.toJSON {
        bwrap = "${pkgs.bubblewrap}/bin/bwrap";
        thermal = "${thermal.thermal-cpu}/bin/harbor-cad-thermal";
        thermal_closure = "${pkgs.closureInfo {rootPaths = [thermal.thermal-cpu];}}/store-paths";
        contact = "${contact.contact-reference-cpu}/bin/harbor-cad-contact";
        contact_closure = "${pkgs.closureInfo {rootPaths = [contact.contact-reference-cpu];}}/store-paths";
        cad = null;
        openlb = null;
        openlb_backend = "cpu";
        render = null;
        video = null;
        filter = null;
      });
      inherit (native) cad-mesh-fixtures runtime-cad-fixtures runtime-cad-only;
      runtime-filter-hip = pkgs.writeText "harbor-cad-native-runtime.json" (builtins.toJSON {
        bwrap = "${pkgs.bubblewrap}/bin/bwrap";
        cad = null;
        openlb = null;
        openlb_backend = "hip";
        render = null;
        video = null;
        filter = "${filters.filter-hip}/bin/harbor-cad-filter";
      });
      gui = pkgs.freecad;
    };
    apps.${system} = {
      default = {
        type = "app";
        program = "${cli}/bin/harbor-cad";
      };
      mcp = {
        type = "app";
        program = "${mcp}/bin/harbor-cad-mcp";
      };
    };
    checks.${system} = {
      rust = cli;
      policy = pkgs.runCommand "harbor-cad-policy" {} ''
        test '${toString pkgs.config.allowUnfree}' = ""
        test '${toString cudaPkgs.config.allowUnfree}' = ""
        test '${fleetix.rev}' = '2230d9ee804a66d94424a91919182e4fcca13ab2'
        test '${builtins.hashString "sha256" (builtins.readFile (fleetix + "/lib/generated/gpu-contract.json"))}' = "$(${cli}/bin/harbor-cad doctor | ${pkgs.jq}/bin/jq -r .fleetix_contract_digest)"
        mkdir $out
      '';
      clean-runtime = pkgs.runCommand "harbor-cad-clean-runtime" {nativeBuildInputs = [pkgs.coreutils];} ''
        mkdir -p home
        env -i HOME=$PWD/home ${cli}/bin/harbor-cad doctor > doctor.json
        env -i HOME=$PWD/home ${cli}/bin/harbor-cad schema > schemas.json
        env -i HOME=$PWD/home ${mcp}/bin/harbor-cad-mcp --help > mcp.txt
        mkdir $out; cp doctor.json schemas.json mcp.txt $out/
      '';
      python = pkgs.runCommand "harbor-cad-python-tests" {nativeBuildInputs = [pkgs.stdenv.cc];} ''
        export HOME=$TMPDIR
        export HARBOR_CAD_TEST_BINARY=${cli}/bin/harbor-cad
        cp -r ${self}/python ./python
        cp -r ${self}/profiles ./profiles
        cp -r ${self}/adapters ./adapters
        cp -r ${self}/scripts ./scripts
        cp -r ${self}/examples ./examples
        # The native CLI/MCP parity fixtures use the same project environment
        # layout as uv; expose the declared test environment in the sandbox.
        chmod u+w python
        ln -s ${testEnv} python/.venv
        ${testEnv}/bin/python -m pytest -q python/tests
        mkdir $out
      '';
      openlb-cpu-reference = pkgs.runCommand "harbor-cad-openlb-cpu-reference" {} ''
        export HOME=$TMPDIR
        ${pkgs.python313}/bin/python ${./scripts/verify_openlb_cpu.py} \
          --executable ${native.openlb-cpu}/bin/harbor-cad-openlb --output "$out"
      '';
    };
    formatter.${system} = format.config.build.wrapper;
    devShells.${system}.default = pkgs.mkShell {packages = [toolchain.rustToolchain pkgs.uv pkgs.python313 format.config.build.wrapper];};
    nixosModules.default = import ./nix/modules.nix {inherit self;};
    homeManagerModules.default = import ./nix/home.nix {inherit self;};
  };
}
