{
  pkgs,
  fem,
}: let
  # General imported solids (docs/enclosure-thermal.md); dispatched by formulation.
  solidBridge = pkgs.replaceVars ../adapters/solid_mesh.py {
    gmsh_version = fem.gmsh.version;
  };
  bridge = pkgs.replaceVars ../adapters/cad_mesh.py {
    fem_bridge = "${fem.bridge}";
    solid_mesh_bridge = "${solidBridge}";
    gmsh_version = fem.gmsh.version;
  };
  adapter = pkgs.writeShellScriptBin "harbor-cad-cad-mesh" ''
    exec ${pkgs.python313}/bin/python3 ${bridge} "$@"
  '';
  closure = pkgs.closureInfo {rootPaths = [adapter];};
in {
  inherit bridge solidBridge;
  cad-mesh-cpu = adapter;
  runtime-cad-mesh-cpu = pkgs.writeText "harbor-cad-cad-mesh-runtime.json" (builtins.toJSON {
    schema_version = 1;
    bwrap = "${pkgs.bubblewrap}/bin/bwrap";
    cad_mesh = "${adapter}/bin/harbor-cad-cad-mesh";
    backend = "cpu";
    qualification = "unqualified";
  });
  runtime-cad-mesh-worker = pkgs.writeText "harbor-cad-native-runtime.json" (builtins.toJSON {
    bwrap = "${pkgs.bubblewrap}/bin/bwrap";
    cad_mesh = "${adapter}/bin/harbor-cad-cad-mesh";
    cad_mesh_closure = "${closure}/store-paths";
    cad = null;
    openlb = null;
    openlb_backend = "cpu";
    render = null;
    video = null;
  });
}
