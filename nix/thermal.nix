{
  pkgs,
  fem,
}: let
  # Nodal temperature serialization only; equations and source pin unchanged.
  # The previously qualified static FEM package remains independent.
  calculix = fem.calculix.overrideAttrs (old: {
    patches = (old.patches or []) ++ [./patches/calculix-temperature-precision.patch];
  });
  bridge = pkgs.replaceVars ../adapters/thermal_history.py {
    fem_bridge = "${fem.bridge}";
    calculix = "${calculix}/bin/ccx";
    ccx_version = calculix.version;
    temperature_patch_sha256 = builtins.hashFile "sha256" ./patches/calculix-temperature-precision.patch;
    gmsh_version = fem.gmsh.version;
  };
  adapter = pkgs.writeShellScriptBin "harbor-cad-thermal" ''
    exec ${pkgs.python313}/bin/python3 ${bridge} "$@"
  '';
  closure = pkgs.closureInfo {rootPaths = [adapter];};
in {
  inherit calculix;
  thermal-cpu = adapter;
  runtime-thermal-cpu = pkgs.writeText "harbor-cad-thermal-runtime.json" (builtins.toJSON {
    schema_version = 1;
    bwrap = "${pkgs.bubblewrap}/bin/bwrap";
    thermal = "${adapter}/bin/harbor-cad-thermal";
    thermal_closure = "${closure}/store-paths";
    backend = "cpu";
    qualification = "unqualified";
  });
  runtime-thermal-worker = pkgs.writeText "harbor-cad-native-runtime.json" (builtins.toJSON {
    bwrap = "${pkgs.bubblewrap}/bin/bwrap";
    thermal = "${adapter}/bin/harbor-cad-thermal";
    thermal_closure = "${closure}/store-paths";
    cad = null;
    openlb = null;
    openlb_backend = "cpu";
    render = null;
    video = null;
    filter = null;
  });
}
