# Design conditions and surface finishes

`data/conditions.json` holds the outdoor climatic classes, external film
coefficients and conservative electronics defaults. `data/finishes.json`
holds surface finishes. Both load through
`conditions::Conditions::builtin()`, in the same way as the
[material and process library](materials-and-cost.md). The loader enforces the
following rules:

- Ids are unique tokens.
- Every known value is non-synthetic and cites a URL and an access date.
- Units normalize through `Quantity::si`, ranges have low ≤ high and stay
  inside physical bounds, and each class has low ≤ high temperature and
  humidity.
- Each class's `wind_film` is at that class's wind speed.
- Finish `compatible_materials` name known opaque library materials.

Unsourced values stay `missing`.

## Climatic classes

The classes come from ETSI EN 300 019-1-4 V2.2.1 (2014-04), clause 5.1 Table 1
(free PDF from etsi.org). Non-weatherprotected locations:

| Class | Low air | High air | Low RH | Solar | Air movement | IEC 60721-3-4 basis |
|---|---|---|---|---|---|---|
| `etsi-4-1` (4.1, most of Europe) | -33 °C | +40 °C | 15 % | 1120 W/m² | 50 m/s | 4K2/… |
| `etsi-4-1e` (4.1E, extended) | -45 °C | +45 °C | 8 % | 1120 W/m² | 50 m/s | none (clause 4.2 note) |
| `etsi-4-2l` (4.2L, extremely cold) | -65 °C | +35 °C | 20 % | 1120 W/m² | 50 m/s | 4K4L/… |
| `etsi-4-2h` (4.2H, extremely warm dry) | -20 °C | +55 °C | 4 % | 1120 W/m² | 50 m/s | 4K4H/… |

High relative humidity is 100 % for every class. The 1120 W/m² solar value
matches IEC 60068-2-5:2018 test Sa, which uses an AM1.5 spectrum over
290–2500 nm. Only the contents list of the IEC preview is public, so the
irradiance is cross-checked against a test-lab summary and the MIL-STD-810
Method 505 peak of 1120 W/m².

## Film coefficients

These are convective coefficients only. Radiation is modelled separately.

- `natural-still-air`, 2.2–5.7 W/(m²·K). Laminar Churchill–Chu correlation
  for a vertical plate (Incropera eq. 9.27), with air properties from Çengel
  Table A-9. Inputs: L = 0.1–0.5 m, surface-to-air ΔT = 5–30 K, film
  temperature 20–80 °C, giving Ra = 3.2e6–2.8e7. The computed range is
  2.26–5.65 W/(m²·K), rounded outward.
- `forced-wind-50`, 86–188 W/(m²·K). Average flat-plate Nusselt number, laminar
  (Incropera eq. 7.30) or mixed (eq. 7.38, Re_cr = 5e5). Inputs: the class air
  movement of 50 m/s, L = 0.1–0.5 m and film temperature -100…+20 °C, giving
  Re = 3.3e5–4.3e6. The computed range is 86.4–187.8 W/(m²·K).

- `enclosed-cavity-natural` (section `internal_films`), 1.2–8.2 W/(m²·K).
  Natural convection on the cavity surfaces of a small closed electronics
  enclosure. Laminar Churchill–Chu vertical plate (all Ra) and McAdams
  heated-face-down plate Nu = 0.27 Ra^¼ (only where 1e5 ≤ Ra ≤ 1e10; Simons,
  Electronics Cooling 2001), with Çengel Table A-9 air. Inputs: L = 0.02–0.15 m,
  surface-to-air ΔT = 2–20 K, film temperature 40–100 °C, giving
  Ra = 5.6e2–5.3e6. Vertical gives 2.48–8.17 and face-down 1.23–3.03
  W/(m²·K), rounded outward. The low end is the design value: cavity radiation
  and gap conduction are not credited, so it is conservative for interior air
  and board temperatures. Read it as `film:enclosed-cavity-natural.low`.

## Electronics defaults

Section `electronics` holds cited conservative defaults. They are not product
data: replace them with the limiting part's datasheet and the real stack-up
once those are known. Read them as `electronics:<id>.<low|high>`.

| Id | Range | Design value | Source |
|---|---|---|---|
| `component-limit-industrial` | 85 °C | `.low` = 358.15 K | Microchip AN3539 ("standard industrial range of -40 °C to +85 °C"); Microchip dsPIC33A AEC-Q100 Grade 3 (-40 to +85 °C) |
| `pcb-fr4-copper-in-plane` | 8.1–17.4 W/(m·K) | `.low` = 8.1 W/(m·K) (orthotropic board) | Graebner, Electronics Cooling 1995 (measured 8.1–15.9); Electronics Cooling 1998 (4-layer example 17.4) |
| `pcb-fr4-copper-through-plane` | 0.26–0.81 W/(m·K) | `.low` = 0.26 W/(m·K) | Electronics Cooling 1998 (series rule 0.26); Graebner 1995 (measured 0.64–0.81) |

The 85 °C grade is an ambient operating range, and grade codes are
manufacturer-specific. Applying the limit to the hotter of the board peak and
the internal air is conservative for a part graded industrial or wider. The
pilot board is orthotropic: 0.26 W/(m·K) along its normal and 8.1 W/(m·K) in
plane, both low ends. 0.26 lies below every cited value, so it is a bound in
any direction. 8.1 bounds in-plane conduction only for boards with continuous
copper planes (Graebner measured 0.81 without them), which is a board
requirement; a board without planes must use the isotropic 0.26.

## Design points

`design_point(class, Point::Hot | Point::Cold)` returns SI values with their
citation trail:

| Point | Ambient | Solar | Film |
|---|---|---|---|
| Hot | high end of the high air temperature | peak class solar (1120 W/m²) | low end of `natural-still-air` (2.2) |
| Cold | low end of the low air temperature | 0 (night) | high end of the class `wind_film` (188) |

For class 4.1, the hot point is 313.15 K, 1120 W/m² and 2.2 W/(m²·K). The cold
point is 240.15 K, 0 W/m² and 188 W/(m²·K).

Each point combines the worst cases. At the hot point the peak air
temperature, full sun and still air coincide, and still air gives the lowest
convective film. At the cold point there is no sun and the strongest wind,
which pulls a powered enclosure towards ambient. With no power, steady state
is ambient for any film value.

The film is a constant `h`, not iterated on ΔT or wind
(`ponytail: conservative constant h; upgrade to ΔT/wind-dependent correlation
iteration`).

## Finishes

`surface_optics(material, finish)` returns (solar absorptance, emissivity). A
known finish value overrides the material's value, and a missing one falls back
to it. `bare` applies no finish, so the material's own values are returned.
Transparent glazing (`pc`, `pc-uv`, `pmma`) is refused with every finish. Its
solar load is transmittance, not surface absorption, so no absorbed-solar
fallback exists for it; the thermal model's explicit `transparent_solar`
conservative bound is the only route (see [enclosure thermal](enclosure-thermal.md)). Design keys `finish:<id>.solar_absorptance` (high end)
and `finish:<id>.emissivity` (low end) return the conservative ends;
`film:<id>.<low|high>` returns the named end of any film.

| Finish | α_s | ε | Source | Compatible |
|---|---|---|---|---|
| `powder-coat-white` (RAL 9016) | 0.20–0.22 | 0.82–0.92 | IGP TSR figure; NASA white paints for ε | Al, ss304 |
| `powder-coat-light-grey` (RAL 7035) | 0.43–0.47 | 0.82–0.94 | IGP TSR figure; NASA paints for ε | Al, ss304 |
| `powder-coat-dark-grey` (RAL 7016) | 0.91–0.92 | 0.82–0.94 | IGP TSR table | Al, ss304 |
| `powder-coat-black` (RAL 9005) | 0.94–0.96 | 0.84–0.94 | IGP TSR figure; NASA black paints | Al, ss304 |
| `anodize-clear` | 0.27–0.35 | 0.76–0.84 | NASA RP-1121 | Al |
| `anodize-black` | 0.65–0.88 | 0.82–0.88 | NASA RP-1121 | Al |
| `pigment-black-polymer` | 0.93–0.96 | 0.85–0.92 | NASA RP-1121 black plastics | PA12, ASA, PU |
| `pigment-white-polymer` | missing | missing | no polymer-specific source | PA12, ASA, PU |
| `bare` | material | material | — | all opaque |

The powder-coat absorptances are α = 1 − TSR, from IGP-HWFclassic 59
measurements (ASTM G197-14, AM1.5, on EN AW-5005). The figure values are read
off the chart to ±0.01–0.02. No separate emittance source was found for
polyester powder binders, so the powder-coat emissivities use NASA organic
paint emittance by analogy. Black-polymer values come from POM, PE, PVF and
Velostat, also by analogy. No pigmented or natural PC finish is listed, because
the library has only clear PC.

## Limits

- Standard classes are design severities for test and classification. They
  are not site weather. A site with a hotter, colder or windier record needs
  its own cited class.
- Table 1 note 1: clear-night sky radiation can cool exposed surfaces 10–20 °C
  below air temperature. The cold point does not include this, and the thermal
  model has no sky temperature.
- The 50 m/s air movement is a mechanical/structural severity. It is used here
  only as an upper bound for the cold-point film.
- NASA and IGP values are laboratory initial values. Soiling, chalking and UV
  ageing raise absorptance and are not covered.
- The finish data apply to the outer surface only. Internal optics and
  conduction through the coating are not modelled.
