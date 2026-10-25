{
  pkgs,
  cudaPkgs,
  inputs,
}: let
  inherit (pkgs) lib;
  freecad = assert lib.versionAtLeast pkgs.freecad.version "1.1.4"; pkgs.freecad;
  adapter = name: executable: script:
    pkgs.writeShellScriptBin name ''
      exec ${executable} ${script} "$@"
    '';
  importPolicy = pkgs.runCommand "harbor-cad-import-policy" {} ''
    mkdir -p $out
    cp ${../adapters/import_policy.py} $out/harbor_cad_import_policy.py
    cp ${../adapters/cad_variant.py} $out/harbor_cad_cad_variant.py
    cp ${../adapters/enclosure_fixture.py} $out/harbor_cad_enclosure_fixture.py
  '';
  cadBridge = pkgs.replaceVars ../adapters/freecad_bridge.py {policy_dir = "${importPolicy}";};
  cad = pkgs.writeShellScriptBin "harbor-cad-import" ''
    export HARBOR_CAD_OPERATION="$1" HARBOR_CAD_PLAN="$2"
    exec ${freecad}/bin/FreeCADCmd --safe-mode ${cadBridge}
  '';
  cadClosure = pkgs.closureInfo {rootPaths = [cad];};
  cadFixtures = adapter "harbor-cad-cad-mesh-fixtures" "${freecad}/bin/FreeCADCmd --safe-mode" ../adapters/qualification_cad_boxes.py;
  # Stock Hydra-built ParaView with external VTK (EGL and X windows selected at
  # runtime). The unwrapped interpreter avoids the Qt wrapper's QT_* paths.
  # ponytail: stock build carries Qt/MPI; switch to a nixpkgs headless variant
  # once one exists.
  visualization = adapter "harbor-cad-render" "${pkgs.paraview}/bin/.pvpython-wrapped" (pkgs.replaceVars ../adapters/paraview_bridge.py {
    libegl = "${pkgs.libglvnd}/lib/libEGL.so.1";
  });
  media = adapter "harbor-cad-video" "${pkgs.python313}/bin/python3" (pkgs.replaceVars ../adapters/video_bridge.py {
    ffmpeg = "${pkgs.ffmpeg}/bin/ffmpeg";
    ffprobe = "${pkgs.ffmpeg}/bin/ffprobe";
  });
  mkOpenlb = backend: import ./openlb.nix {inherit pkgs cudaPkgs inputs backend;};
  openlb-cpu = mkOpenlb "cpu";
  openlb-cuda = mkOpenlb "cuda";
  openlb-hip = mkOpenlb "hip";
  runtime = backend: openlb:
    pkgs.writeText "harbor-cad-native-runtime.json" (builtins.toJSON {
      bwrap = "${pkgs.bubblewrap}/bin/bwrap";
      cad = "${cad}/bin/harbor-cad-import";
      cad_closure = "${cadClosure}/store-paths";
      # CPU references need only CAD and flow; rendering/media remain separately
      # selected closures, avoiding a mandatory ParaView build for this slice.
      render =
        if backend == "cpu"
        then null
        else "${visualization}/bin/harbor-cad-render";
      video =
        if backend == "cpu"
        then null
        else "${media}/bin/harbor-cad-video";
      inherit openlb;
      openlb_backend = backend;
    });
in {
  inherit cad visualization media openlb-cpu openlb-cuda openlb-hip;
  cad-mesh-fixtures = cadFixtures;
  runtime-cad-only = pkgs.writeText "harbor-cad-cad-runtime.json" (builtins.toJSON {
    bwrap = "${pkgs.bubblewrap}/bin/bwrap";
    cad = "${cad}/bin/harbor-cad-import";
    cad_closure = "${cadClosure}/store-paths";
    openlb = null;
    openlb_backend = "cpu";
    render = null;
    video = null;
  });
  runtime-cad-fixtures = pkgs.writeText "harbor-cad-cad-fixture-runtime.json" (builtins.toJSON {
    schema_version = 1;
    bwrap = "${pkgs.bubblewrap}/bin/bwrap";
    fixture = "${cadFixtures}/bin/harbor-cad-cad-mesh-fixtures";
    qualification = "unqualified";
  });
  runtime-cpu = runtime "cpu" "${openlb-cpu}/bin/harbor-cad-openlb";
  runtime-cuda = runtime "cuda" "${openlb-cuda}/bin/harbor-cad-openlb";
  runtime-hip = runtime "hip" "${openlb-hip}/bin/harbor-cad-openlb";
}
