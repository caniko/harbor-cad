{
  pkgs,
  inputs,
  cudaPkgs,
  backend ? "cpu",
}: let
  inherit (pkgs) lib;
  # The driver implements CPU and HIP device identity only; CUDA would need its
  # own PCI selection and receipt code, so no CUDA wetting package is offered.
  base = assert builtins.elem backend ["cpu" "hip"];
    import ./openlb.nix {inherit pkgs inputs cudaPkgs backend;};
  native = base.overrideAttrs (old: {
    pname = "harbor-cad-openlb-wetting-${backend}";
    patches = old.patches ++ [./patches/openlb-wetting-initial-center.patch];
    preBuild =
      old.preBuild
      + ''
        cp ${../adapters/openlb_wetting.cpp} harbor-driver/harbor-cad-openlb.cpp
        cp ${../adapters/wetting_output.hpp} harbor-driver/wetting_output.hpp
      ''
      + lib.optionalString (backend == "hip") ''
        substituteInPlace harbor-driver/harbor-cad-openlb.cpp \
          --replace-fail '@hip_architecture@' '${old.passthru.hipArchitecture}'
      '';
    installPhase = ''
      mkdir -p $out/bin $out/share/licenses/openlb
      cp harbor-driver/harbor-cad-openlb $out/bin/harbor-cad-openlb-wetting
      cp COPYING $out/share/licenses/openlb/ 2>/dev/null || cp LICENSE* $out/share/licenses/openlb/
    '';
    passthru =
      old.passthru
      // {
        formulation = "well_balanced_contact_angle_2d";
        dimensionality = 2;
      };
  });
  femBridge = pkgs.writeText "fem_reference.py" (builtins.readFile ../adapters/fem_reference.py);
  bridge = pkgs.replaceVars ../adapters/wetting_reference.py {
    fem_bridge = femBridge;
    native = "${native}/bin/harbor-cad-openlb-wetting";
  };
  adapter = pkgs.writeShellScriptBin "harbor-cad-wetting" ''
    exec ${pkgs.python313}/bin/python3 -B ${bridge} "$@"
  '';
  closure = pkgs.closureInfo {rootPaths = [adapter];};
in {
  "wetting-native-${backend}" = native;
  "wetting-reference-${backend}" = adapter;
  "runtime-wetting-reference-${backend}" = pkgs.writeText "harbor-cad-wetting-reference-runtime.json" (builtins.toJSON {
    schema_version = 1;
    bwrap = "${pkgs.bubblewrap}/bin/bwrap";
    wetting = "${adapter}/bin/harbor-cad-wetting";
    wetting_closure = "${closure}/store-paths";
    inherit backend;
    qualification = "unqualified";
  });
  # The CPU worker runtime keeps its historical output name.
  "runtime-wetting-worker${lib.optionalString (backend != "cpu") "-${backend}"}" = pkgs.writeText "harbor-cad-native-runtime.json" (builtins.toJSON {
    bwrap = "${pkgs.bubblewrap}/bin/bwrap";
    wetting = "${adapter}/bin/harbor-cad-wetting";
    wetting_closure = "${closure}/store-paths";
    cad = null;
    openlb = null;
    openlb_backend = backend;
    render = null;
    video = null;
    filter = null;
  });
}
