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
  # Internal VTK comes from the exact ParaView source archive, avoiding an ABI mix.
  paraview = pkgs.stdenv.mkDerivation {
    pname = "harbor-cad-paraview-egl";
    inherit (pkgs.paraview) version src;
    nativeBuildInputs = [pkgs.cmake pkgs.ninja];
    buildInputs = [pkgs.python313 pkgs.mesa pkgs.libGL pkgs.libglvnd pkgs.zlib];
    cmakeFlags = [
      # Bundled VTK rejects absolute GNUInstallDirs from the Nix CMake hook.
      # Match the relative destinations in the pinned Nixpkgs ParaView recipe.
      "-DCMAKE_INSTALL_BINDIR=bin"
      "-DCMAKE_INSTALL_LIBDIR=lib"
      "-DCMAKE_INSTALL_INCLUDEDIR=include"
      "-DCMAKE_INSTALL_DOCDIR=share/paraview/doc"
      "-DPARAVIEW_USE_QT=OFF"
      "-DPARAVIEW_USE_MPI=OFF"
      "-DPARAVIEW_USE_PYTHON=ON"
      "-DPARAVIEW_USE_EXTERNAL_VTK=OFF"
      "-DPARAVIEW_VERSIONED_INSTALL=OFF"
      "-DPARAVIEW_ENABLE_WEB=OFF"
      "-DPARAVIEW_ENABLE_CATALYST=OFF"
      "-DPARAVIEW_BUILD_TESTING=OFF"
      "-DBUILD_TESTING=OFF"
      "-DVTK_BUILD_TESTING=OFF"
      "-DVTK_USE_X=OFF"
      "-DVTK_OPENGL_HAS_EGL=ON"
      "-DVTK_DEFAULT_RENDER_WINDOW_OFFSCREEN=ON"
      "-DVTK_SMP_IMPLEMENTATION_TYPE=Sequential"
      "-DVTK_USE_EXTERNAL=OFF"
    ];
    enableParallelBuilding = true;
    doInstallCheck = true;
    installCheckPhase = ''
      env -i HOME=$TMPDIR $out/bin/pvpython -c 'from paraview import simple; from vtkmodules.vtkRenderingOpenGL2 import vtkEGLRenderWindow; assert vtkEGLRenderWindow.__name__ == "vtkEGLRenderWindow"'
    '';
    meta.license = lib.licenses.bsd3;
  };
  visualization = adapter "harbor-cad-render" "${paraview}/bin/pvpython" (pkgs.replaceVars ../adapters/paraview_bridge.py {
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
