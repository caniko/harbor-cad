"""CI-safe contract for the stock-ParaView wiring.

CI has no Nix and no GPU, so these read the declarations directly: the render
adapter must run the stock unwrapped ``pvpython``, the cache pin must only move
to nixpkgs revisions whose ``paraview`` binary is cached, and the clean-env EGL
import check must stay wired.
"""

from pathlib import Path

REPO = Path(__file__).parents[2]


def native():
    return (REPO / "nix/native.nix").read_text()


def flake():
    return (REPO / "flake.nix").read_text()


def test_render_adapter_runs_the_unwrapped_stock_pvpython():
    source = native()
    # The Qt wrapper would inject QT_* paths; the adapter runs the binary the
    # wrapper wraps.
    assert 'lib.getExe\' pkgs.paraview ".pvpython-wrapped"' in source
    assert 'adapter "harbor-cad-render" pvpython' in source
    # The local bundled-VTK derivation must not come back.
    assert 'pname = "harbor-cad-paraview-egl"' not in source
    assert "PARAVIEW_USE_EXTERNAL_VTK=OFF" not in source


def test_clean_environment_egl_import_check_stays_wired():
    assert "paraview-egl-import = pkgs.runCommand" in native()
    assert "env -i HOME=$PWD/home" in native()
    assert "inherit (native) paraview-egl-import" in flake()


def test_cache_pin_covers_stock_paraview():
    source = flake()
    assert (
        'packages = ["freecad" "paraview" "rocmPackages.clr" '
        '"rocmPackages.rocthrust" "rocmPackages.rocprim"]' in source
    )
    assert "inherit (native) paraview openlb-cpu" not in source
