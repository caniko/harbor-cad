"""Deterministic synthetic outdoor-enclosure seed built from native Part primitives.

Every dimension is an expression on the single `Params` spreadsheet, so a
version-2 sheet variant changes geometry only through existing alias cells.
No Python feature proxies are created; the saved document holds C++ objects only.
"""

PARAMS = (
    ("length", "200 mm"),
    ("width", "150 mm"),
    ("height", "80 mm"),
    ("wall", "4 mm"),
    ("fin_count", "8"),
    ("fin_height", "15 mm"),
    ("fin_pitch", "20 mm"),
    ("fin_thickness", "2.5 mm"),
    ("groove_width", "2 mm"),
    ("groove_depth", "1.5 mm"),
    ("lid_overlap", "10 mm"),
    ("window_width", "60 mm"),
    ("window_height", "40 mm"),
    ("drain_diameter", "6 mm"),
    ("boss_count", "4"),
    ("pcb_power_area_side", "60 mm"),
)
REGIONS = ("housing", "lid", "gasket", "window", "pcb", "mount")
# ponytail: fixed fin/boss slots; counts above these clamp silently because an
# unused slot is a tiny primitive absorbed inside the wall/floor. Add slots if
# the design space needs more fins or bosses.
FIN_SLOTS = 16
BOSS_SLOTS = 4

L, W, H, T = "Params.length", "Params.width", "Params.height", "Params.wall"
GW, GD = "Params.groove_width", "Params.groove_depth"
SIDE = "Params.pcb_power_area_side"


def build(doc):
    sheet = doc.addObject("Spreadsheet::Sheet", "Params")
    for row, (alias, value) in enumerate(PARAMS, start=1):
        sheet.set(f"A{row}", alias)
        sheet.set(f"B{row}", value)
        sheet.setAlias(f"B{row}", alias)

    def bind(obj, expressions):
        for prop, expression in expressions.items():
            if expression is not None:
                obj.setExpression(prop, expression)
        return obj

    def place(at):
        return {f"Placement.Base.{a}": e for a, e in zip("xyz", at, strict=True)}

    def box(name, size, at=(None, None, None)):
        obj = doc.addObject("Part::Box", name)
        return bind(obj, dict(zip(("Length", "Width", "Height"), size)) | place(at))

    def cylinder(name, radius, height, at):
        obj = doc.addObject("Part::Cylinder", name)
        return bind(obj, {"Radius": radius, "Height": height} | place(at))

    def boolean(kind, name, base, tool, label=None):
        obj = doc.addObject(f"Part::{kind}", name)
        obj.Base, obj.Tool = base, tool
        obj.Label = label or name
        return obj

    def ring(prefix, height, z, cut_height, cut_z):
        outer = box(
            f"{prefix}_outer",
            (f"{L} - {T} + {GW}", f"{W} - {T} + {GW}", height),
            (f"{T} / 2 - {GW} / 2", f"{T} / 2 - {GW} / 2", z),
        )
        inner = box(
            f"{prefix}_inner",
            (f"{L} - {T} - {GW}", f"{W} - {T} - {GW}", cut_height),
            (f"{T} / 2 + {GW} / 2", f"{T} / 2 + {GW} / 2", cut_z),
        )
        return outer, inner

    shell = boolean(
        "Cut",
        "shell",
        box("outer", (L, W, H)),
        box("cavity", (f"{L} - 2 * {T}", f"{W} - 2 * {T}", H), (T, T, T)),
    )
    parts = [shell]
    for i in range(FIN_SLOTS):
        used = f"Params.fin_count > {i}"

        def either(active, absorbed, used=used):
            return f"({used}) ? ({active}) : ({absorbed})"

        parts.append(
            box(
                f"fin{i}",
                (
                    either("Params.fin_thickness", f"{T} / 4"),
                    either(f"Params.fin_height + {T} / 2", f"{T} / 4"),
                    either(f"{H} - Params.lid_overlap - 2 * {T}", f"{T} / 4"),
                ),
                (
                    either(
                        f"({L} - (Params.fin_count - 1) * Params.fin_pitch) / 2"
                        f" + {i} * Params.fin_pitch - Params.fin_thickness / 2",
                        f"{L} / 2",
                    ),
                    either(f"{W} - {T} / 2", f"{W} - 3 * {T} / 4"),
                    either(T, f"{H} / 2"),
                ),
            )
        )
    offset = f"{SIDE} / 2 - 5 mm"
    for i, (sx, sy) in enumerate(((-1, -1), (1, -1), (1, 1), (-1, 1))[:BOSS_SLOTS]):
        used = f"Params.boss_count > {i}"
        parts.append(
            cylinder(
                f"boss{i}",
                "3 mm",
                f"({used}) ? (9 mm) : ({T} / 4)",
                (
                    f"{L} / 2 + {sx} * ({offset})",
                    f"{W} / 2 + {sy} * ({offset})",
                    f"({used}) ? ({T} - 1 mm) : ({T} / 4)",
                ),
            )
        )
    body = doc.addObject("Part::MultiFuse", "body")
    body.Shapes = parts
    groove = boolean(
        "Cut",
        "groove",
        *ring(
            "groove",
            f"{GD} + 1 mm",
            f"{H} - {GD}",
            f"{GD} + 2 mm",
            f"{H} - {GD} - 1 mm",
        ),
    )
    drain = cylinder(
        "drain",
        "Params.drain_diameter / 2",
        f"{T} + 2 mm",
        (f"{L} / 2", f"{T} + Params.drain_diameter + 2 mm", "-1 mm"),
    )
    boolean(
        "Cut",
        "housing",
        boolean("Cut", "grooved", body, groove),
        drain,
        "housing shell with back fins and drain",
    )
    boolean(
        "Cut",
        "gasket",
        *ring("gasket", GD, f"{H} - {GD}", f"{GD} + 2 mm", f"{H} - {GD} - 1 mm"),
        "gasket ring in rim groove",
    )
    aperture_at = (
        f"{L} / 2 - Params.window_width / 2",
        f"{W} / 2 - Params.window_height / 2",
    )
    lid_shell = boolean(
        "Cut",
        "lid_shell",
        box(
            "lid_outer",
            (f"{L} + 2 * {T}", f"{W} + 2 * {T}", f"{T} + Params.lid_overlap"),
            (f"-{T}", f"-{T}", f"{H} - Params.lid_overlap"),
        ),
        box(
            "lid_inner",
            (L, W, "Params.lid_overlap + 1 mm"),
            (None, None, f"{H} - Params.lid_overlap - 1 mm"),
        ),
    )
    boolean(
        "Cut",
        "lid",
        lid_shell,
        box(
            "aperture",
            ("Params.window_width", "Params.window_height", f"{T} + 2 mm"),
            (*aperture_at, f"{H} - 1 mm"),
        ),
        "lid with overlapping skirt and window aperture",
    )
    box(
        "window", ("Params.window_width", "Params.window_height", T), (*aperture_at, H)
    ).Label = "window pane"
    box(
        "pcb",
        (SIDE, SIDE, "2 mm"),
        (f"{L} / 2 - {SIDE} / 2", f"{W} / 2 - {SIDE} / 2", f"{T} + 8 mm"),
    ).Label = "pcb heat source"
    mount = doc.addObject("Part::MultiFuse", "mount")
    mount.Label = "wall bracket"
    mount.Shapes = [
        box(
            "mount_base",
            (f"{L} * 0.6", f"{W} / 2 + Params.fin_height + 10 mm + {T}", T),
            (f"{L} * 0.2", f"{W} / 2", f"-{T}"),
        ),
        box(
            "mount_back",
            (f"{L} * 0.6", T, f"{H} * 0.75 + {T}"),
            (f"{L} * 0.2", f"{W} + Params.fin_height + 10 mm", f"-{T}"),
        ),
    ]
    doc.recompute()
    for obj in doc.Objects:
        if not obj.isValid() or "Invalid" in obj.State or "Touched" in obj.State:
            raise ValueError(f"synthetic enclosure object {obj.Name} failed recompute")
    for name in REGIONS:
        shape = doc.getObject(name).Shape
        if shape.isNull() or not shape.isValid() or len(shape.Solids) != 1:
            raise ValueError(f"synthetic enclosure region {name} must be one solid")
    return sheet
