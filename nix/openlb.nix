{
  pkgs,
  cudaPkgs,
  inputs,
  backend,
  hipArchitecture ? "gfx1100",
}: let
  inherit (pkgs) lib;
  inherit (cudaPkgs) cudaPackages;
  cuda = backend == "cuda";
  hip = backend == "hip";
  # clr supplies the Nix-wrapped hipcc and its matching headers/runtime.
  rocm = pkgs.rocmPackages.clr;
in
  assert builtins.elem backend ["cpu" "cuda" "hip"];
  assert !hip || builtins.elem hipArchitecture rocm.gpuTargets;
    pkgs.stdenv.mkDerivation {
      pname = "harbor-cad-openlb-${backend}";
      version = "1.9.0";
      src = inputs.openlb;
      patches =
        [./patches/openlb-vtk-output-precision.patch]
        # Without MPI, upstream HIP overlap copies are never awaited, so stream()
        # can race the periodic/overlap copies.
        ++ lib.optionals hip [./patches/openlb-hip-dependencies.patch ./patches/openlb-hip-communicator-sync.patch];
      nativeBuildInputs = [pkgs.gnumake] ++ lib.optionals cuda [cudaPackages.cuda_nvcc] ++ lib.optionals hip [rocm];
      buildInputs = [pkgs.zlib pkgs.tinyxml-2 pkgs.nlohmann_json] ++ lib.optionals cuda [cudaPackages.cuda_cudart] ++ lib.optionals hip [rocm pkgs.rocmPackages.rocthrust pkgs.rocmPackages.rocprim];
      dontConfigure = true;
      preBuild = ''
        cat > config.mk <<'EOF'
        CXX := ${
          if cuda
          then "nvcc"
          else if hip
          then "hipcc"
          else "g++"
        }
        CC := gcc
        CXXFLAGS := -O2 -std=c++20 -I${pkgs.nlohmann_json}/include -I${pkgs.tinyxml-2}/include -I${lib.getDev pkgs.zlib}/include ${lib.optionalString hip "-I${pkgs.rocmPackages.rocthrust}/include -I${pkgs.rocmPackages.rocprim}/include"}
        PARALLEL_MODE := NONE
        PLATFORMS := CPU_SISD ${lib.optionalString cuda "GPU_CUDA"} ${lib.optionalString hip "GPU_HIP"}
        CUDA_ARCH := 86
        HIP_PLATFORM := amd
        HIP_ARCH := ${hipArchitecture}
        HIP_CXXFLAGS := -O2 -std=c++20
        HIP_LDFLAGS := --offload-arch=${hipArchitecture} -L${lib.getLib pkgs.zlib}/lib -L${lib.getLib pkgs.tinyxml-2}/lib -Wl,-rpath,${lib.getLib pkgs.zlib}/lib -Wl,-rpath,${lib.getLib pkgs.tinyxml-2}/lib
        FLOATING_POINT_TYPE := double
        USE_EMBEDDED_DEPENDENCIES := OFF
        EOF
        mkdir harbor-driver
        cp ${../adapters/hip_identity.hpp} harbor-driver/hip_identity.hpp
        substituteInPlace harbor-driver/hip_identity.hpp --replace-fail '@hip_architecture@' '${hipArchitecture}'
        cp ${../adapters/openlb.cpp} harbor-driver/harbor-cad-openlb.cpp
        substituteInPlace harbor-driver/harbor-cad-openlb.cpp \
          --replace-fail '@hip_architecture@' '${hipArchitecture}'
        cat > harbor-driver/Makefile <<'EOF'
        OLB_ROOT := ..
        EXAMPLE := harbor-cad-openlb
        include $(OLB_ROOT)/default.mk
        EOF
        ${lib.optionalString cuda ''
          # Stubs are link-time support only, never an admitted runtime driver.
          echo 'CUDA_LDFLAGS := -L${lib.getLib cudaPackages.cuda_cudart}/lib -L${lib.getLib cudaPackages.cuda_cudart}/lib/stubs' >> config.mk
        ''}
      '';
      buildPhase = ''
        runHook preBuild
        make -C harbor-driver -j2
        runHook postBuild
      '';
      installPhase = ''
        mkdir -p $out/bin $out/share/licenses/openlb
        cp harbor-driver/harbor-cad-openlb $out/bin/
        cp COPYING $out/share/licenses/openlb/ 2>/dev/null || cp LICENSE* $out/share/licenses/openlb/
      '';
      meta.license = lib.licenses.gpl2Plus;
      passthru = {
        precision = "float64";
        sourceRevision = inputs.openlb.rev;
        cudaArchitecture =
          if cuda
          then "sm_86"
          else null;
        hipArchitecture =
          if hip
          then hipArchitecture
          else null;
        rocmVersion =
          if hip
          then rocm.version
          else null;
        qualification = "unqualified";
      };
    }
