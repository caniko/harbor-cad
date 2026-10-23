"""Fixed native Part::Box allowlist; caller code and property names are never inputs."""

import math


def lengths(request):
    values = []
    for quantity in request["dimensions"]:
        factor = {"m": 1.0, "mm": 0.001, "nm": 1e-9}.get(quantity["unit"])
        if factor is None:
            raise ValueError("explicit SI-capable variant lengths required")
        value = quantity["value"] * factor
        if (
            isinstance(quantity["value"], bool)
            or not math.isfinite(value)
            or not 1e-5 <= value <= 10.0
        ):
            raise ValueError("bounded native box dimensions required")
        values.append(value)
    if len(values) != 3 or max(values) / min(values) > 1000:
        raise ValueError("three resolved bounded box dimensions required")
    return values


def geometry(obj):
    bounds = obj.Shape.BoundBox
    values = {
        "bounds_m": [
            value * 0.001
            for value in (
                bounds.XMin,
                bounds.XMax,
                bounds.YMin,
                bounds.YMax,
                bounds.ZMin,
                bounds.ZMax,
            )
        ],
        "volume_m3": obj.Shape.Volume * 1e-9,
        "transform": list(obj.Placement.toMatrix().A),
        "dimensions_m": [
            float(obj.Length) * 0.001,
            float(obj.Width) * 0.001,
            float(obj.Height) * 0.001,
        ],
    }
    if any(
        not math.isfinite(value)
        for sequence in values.values()
        for value in (sequence if isinstance(sequence, list) else [sequence])
    ):
        raise ValueError("finite native CAD geometry required")
    return values


def apply(doc, spec, tolerance_m):
    """Validate the complete original context before editing only fixed dimensions."""
    name = spec["request"]["region_name"]
    if len(doc.Objects) != 1:
        raise ValueError("single-object native box document required")
    obj = doc.Objects[0]
    if (
        obj.Name != name
        or obj.TypeId != "Part::Box"
        or obj.getParentGeoFeatureGroup() is not None
        or obj.ExpressionEngine
    ):
        raise ValueError(
            "one named top-level native Part::Box without expressions required"
        )
    if obj.Shape.isNull() or not obj.Shape.isValid() or len(obj.Shape.Solids) != 1:
        raise ValueError("one valid native solid required before recompute")
    original = geometry(obj)
    source = spec["source"]["geometry"]
    if (
        original["transform"] != source["source_transform"]
        or any(
            abs(a - b) > tolerance_m
            for a, b in zip(original["bounds_m"], source["bounds_m"], strict=True)
        )
        or abs(original["volume_m3"] - source["volume_m3"])
        > 1e-10 * source["volume_m3"]
    ):
        raise ValueError("native original box differs from registered CAD geometry")
    if any(
        abs(
            original["dimensions_m"][axis]
            - (source["bounds_m"][2 * axis + 1] - source["bounds_m"][2 * axis])
        )
        > tolerance_m
        for axis in range(3)
    ):
        raise ValueError("axis-aligned primitive dimension context required")
    expected = lengths(spec["request"])
    obj.Length, obj.Width, obj.Height = (value * 1000.0 for value in expected)
    doc.recompute()
    if (
        obj.ExpressionEngine
        or obj.Shape.isNull()
        or not obj.Shape.isValid()
        or len(obj.Shape.Solids) != 1
    ):
        raise ValueError(
            "recomputed native variant must remain one valid primitive solid"
        )
    changed = geometry(obj)
    bounds = list(source["bounds_m"])
    for axis in range(3):
        bounds[2 * axis + 1] = bounds[2 * axis] + expected[axis]
    if (
        changed["transform"] != original["transform"]
        or any(
            abs(a - b) > tolerance_m
            for a, b in zip(changed["bounds_m"], bounds, strict=True)
        )
        or any(
            abs(a - b) > tolerance_m
            for a, b in zip(changed["dimensions_m"], expected, strict=True)
        )
        or abs(changed["volume_m3"] - math.prod(expected)) > 1e-10 * math.prod(expected)
    ):
        raise ValueError(
            "recomputed native box differs from approved dimensions or placement"
        )
    return {
        "object_type": "Part::Box",
        "before": original,
        "after": changed,
        "gap_healing": False,
        "expressions": False,
    }


# Version 2: spreadsheet-alias variants. Expressions are the mechanism, so the
# allowlist is "only existing alias cells of the one named sheet change".
SHEET = "Spreadsheet::Sheet"
MIN_WALL_METHOD = (
    "inward ray from the centroid of each face's largest tessellation triangle; "
    "minimum material chord"
)


def sheets(doc):
    """Registered alias evidence for every sheet; values reported in SI."""
    found = []
    for sheet in (o for o in doc.Objects if o.TypeId == SHEET):
        cells = sheet.getUsedCells()
        if len(cells) > 4096:
            raise ValueError("bounded spreadsheet required")
        aliases = {}
        for cell in cells:
            alias = sheet.getAlias(cell)
            if alias:
                aliases[alias] = {"cell": cell, **dimension(sheet.get(alias))}
        if len(aliases) > 256:
            raise ValueError("bounded spreadsheet aliases required")
        found.append({"name": sheet.Name, "label": sheet.Label, "aliases": aliases})
    return {"schema_version": 1, "sheets": found}


def dimension(value):
    unit = getattr(value, "Unit", None)
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        kind, si = "dimensionless", float(value)
    elif unit is not None and unit == type(unit)():
        kind, si = "dimensionless", float(value.Value)
    elif unit is not None and unit == type(value)("1 mm").Unit:
        kind, si = "length", float(value.Value) * 0.001
    else:
        return {"dimension": "other", "value_si": None}
    if not math.isfinite(si):
        raise ValueError("finite spreadsheet alias values required")
    return {"dimension": kind, "value_si": si}


def content(quantity):
    """Fixed FreeCAD quantity text from a validated value/unit; never caller text."""
    value = quantity["value"]
    if isinstance(value, bool) or not math.isfinite(value):
        raise ValueError("finite sheet quantity required")
    if quantity["unit"] == "1":
        if not 0 <= value <= 1e6:
            raise ValueError("bounded dimensionless sheet quantity required")
        return "dimensionless", repr(float(value)), float(value)
    factor = {"m": 1.0, "mm": 0.001, "nm": 1e-9}.get(quantity["unit"])
    if factor is None or not 1e-5 <= value * factor <= 10.0:
        raise ValueError("bounded SI-capable sheet length required")
    return "length", f"{value * factor * 1000.0!r} mm", value * factor


def plain(value):
    if isinstance(value, (list, tuple)):
        return [plain(item) for item in value]
    if hasattr(value, "TypeId") and isinstance(getattr(value, "Name", None), str):
        return "object:" + value.Name
    return repr(value)


def snapshot(doc, sheet):
    """Non-computed state: expression bindings and every unbound input property."""
    state = {}
    for obj in doc.Objects:
        if obj.Name == sheet.Name:
            state[obj.Name] = [
                (cell, obj.getContents(cell), obj.getAlias(cell))
                for cell in sorted(obj.getUsedCells())
            ]
            continue
        bound = {path.lstrip(".").split(".")[0] for path, _ in obj.ExpressionEngine}
        state[obj.Name] = [
            obj.TypeId,
            sorted(obj.ExpressionEngine),
            {
                name: plain(getattr(obj, name))
                for name in obj.PropertiesList
                if name not in bound
                and obj.getTypeIdOfProperty(name) != "Part::PropertyPartShape"
                and not {"Output", "Transient"} & set(obj.getPropertyStatus(name))
            },
        ]
    return state


def healthy(doc):
    for obj in doc.Objects:
        if not obj.isValid() or {"Invalid", "Touched", "Error"} & set(obj.State):
            raise ValueError(f"document object {obj.Name} is invalid or not recomputed")


def region_geometry(obj):
    shape = obj.Shape
    if (
        not obj.isDerivedFrom("Part::Feature")
        or obj.getParentGeoFeatureGroup() is not None
        or shape.isNull()
        or not shape.isValid()
        or len(shape.Solids) != 1
    ):
        raise ValueError("named region must stay one valid top-level native solid")
    box = shape.BoundBox
    values = {
        "bounds_m": [
            v / 1000
            for v in (box.XMin, box.XMax, box.YMin, box.YMax, box.ZMin, box.ZMax)
        ],
        "volume_m3": shape.Volume * 1e-9,
        "transform": list(obj.Placement.toMatrix().A),
    }
    if not all(
        math.isfinite(v)
        for v in [*values["bounds_m"], values["volume_m3"], *values["transform"]]
    ):
        raise ValueError("finite native CAD geometry required")
    return values


def min_wall(shape, part):
    """Smallest material chord along inward face normals, in mm."""
    # ponytail: one probe per face (largest tessellation triangle); a thin spot
    # away from that probe is missed. Upgrade: probe several triangles per face.
    if len(shape.Faces) > 2048:
        raise ValueError("bounded face count required for wall estimate")
    reach = 2.0 * shape.BoundBox.DiagonalLength
    best = math.inf
    for face in shape.Faces:
        points, triangles = face.tessellate(0.1)
        if not triangles:
            continue
        a, b, c = max(
            ((points[i], points[j], points[k]) for i, j, k in triangles),
            key=lambda t: (t[1] - t[0]).cross(t[2] - t[0]).Length,
        )
        probe = (a + b + c) * (1.0 / 3.0)
        normal = face.normalAt(*face.Surface.parameter(probe))
        ray = part.LineSegment(probe, probe - normal * reach).toShape()
        for edge in shape.common(ray).Edges:
            if min((v.Point - probe).Length for v in edge.Vertexes) < 1e-3:
                best = min(best, edge.Length)
    if not math.isfinite(best) or best <= 0:
        raise ValueError("no finite wall thickness probe")
    return best


def apply_sheet(doc, spec, tolerance_m, part):
    """Set only existing alias cells of the one named sheet, then recompute."""
    request = spec["request"]
    found = [o for o in doc.Objects if o.TypeId == SHEET]
    if len(found) != 1 or found[0].Name != request["sheet_name"]:
        raise ValueError("exactly one named Spreadsheet::Sheet required")
    sheet = found[0]
    healthy(doc)
    before = {}
    for region in spec["source"]["regions"]:
        obj = doc.getObject(region["name"])
        if obj is None:
            raise ValueError("registered region missing from original document")
        observed = region_geometry(obj)
        if (
            observed["transform"] != region["transform"]
            or any(
                abs(a - b) > tolerance_m
                for a, b in zip(observed["bounds_m"], region["bounds_m"], strict=True)
            )
            or abs(observed["volume_m3"] - region["volume_m3"])
            > 1e-10 * region["volume_m3"]
        ):
            raise ValueError("native original differs from registered CAD geometry")
        before[region["name"]] = observed
    original = snapshot(doc, sheet)
    changes = {}
    for alias, quantity in sorted(request["parameters"].items()):
        cell = sheet.getCellFromAlias(alias)
        if not cell:
            raise ValueError("only existing spreadsheet aliases may be set")
        kind, text, value = content(quantity)
        if dimension(sheet.get(alias))["dimension"] != kind:
            raise ValueError("sheet value must keep the alias dimension")
        changes[alias] = {"cell": cell, "before": sheet.getContents(cell)}
        sheet.set(cell, text)
        changes[alias]["value"] = value
    edited = snapshot(doc, sheet)
    cells = {change["cell"] for change in changes.values()}
    if any(edited[n] != s for n, s in original.items() if n != sheet.Name) or [
        c for c in edited[sheet.Name] if c[0] not in cells
    ] != [c for c in original[sheet.Name] if c[0] not in cells]:
        raise ValueError("only approved Spreadsheet cells may change")
    doc.recompute()
    healthy(doc)
    if snapshot(doc, sheet) != edited:
        raise ValueError("recompute changed non-computed document state")
    for alias, change in changes.items():
        landed = dimension(sheet.get(alias))["value_si"]
        value = change.pop("value")
        if landed is None or abs(landed - value) > 1e-12 * max(1.0, abs(value)):
            raise ValueError("spreadsheet alias did not take the approved value")
        change["after"] = sheet.getContents(change["cell"])
    after = {}
    for name in request["regions"]:
        obj = doc.getObject(name)
        if obj is None:
            raise ValueError("requested region missing after recompute")
        after[name] = region_geometry(obj) | {
            "surface_area_m2": obj.Shape.Area * 1e-6,
            "min_wall_m": min_wall(obj.Shape, part) * 0.001,
        }
    return {
        "object_type": SHEET,
        "sheet_name": sheet.Name,
        "parameters": changes,
        "only_sheet_cells_changed": True,
        "before": before,
        "after": after,
        "min_wall_method": MIN_WALL_METHOD,
        "gap_healing": False,
        "expressions": True,
    }
