{
  pkgs,
  fem,
  cadMesh,
}: let
  # Version-2 descriptors run the enclosure thermal recipe (docs/enclosure-thermal.md).
  enclosureBridge = pkgs.replaceVars ../adapters/enclosure_thermal.py {
    solid_mesh_bridge = "${cadMesh.solidBridge}";
    calculix = "${fem.calculix}/bin/ccx";
    ccx_version = fem.calculix.version;
    gmsh_version = fem.gmsh.version;
  };
  bridge = pkgs.replaceVars ../adapters/fem_imported.py {
    fem_bridge = "${fem.bridge}";
    cad_mesh_bridge = "${cadMesh.bridge}";
    enclosure_bridge = "${enclosureBridge}";
    calculix = "${fem.calculix}/bin/ccx";
    ccx_version = fem.calculix.version;
    gmsh_version = fem.gmsh.version;
  };
  adapter = pkgs.writeShellScriptBin "harbor-cad-fem-imported" ''
    exec ${pkgs.python313}/bin/python3 ${bridge} "$@"
  '';
  closure = pkgs.closureInfo {rootPaths = [adapter];};
in {
  fem-imported-cpu = adapter;
  runtime-fem-imported-cpu = pkgs.writeText "harbor-cad-fem-imported-runtime.json" (builtins.toJSON {
    schema_version = 1;
    bwrap = "${pkgs.bubblewrap}/bin/bwrap";
    fem_imported = "${adapter}/bin/harbor-cad-fem-imported";
    backend = "cpu";
    qualification = "unqualified";
  });
  runtime-fem-imported-worker = pkgs.writeText "harbor-cad-native-runtime.json" (builtins.toJSON {
    bwrap = "${pkgs.bubblewrap}/bin/bwrap";
    fem_imported = "${adapter}/bin/harbor-cad-fem-imported";
    fem_imported_closure = "${closure}/store-paths";
    cad = null;
    openlb = null;
    openlb_backend = "cpu";
    render = null;
    video = null;
  });
}
