# Conservative one-way transfer foundation

`harbor-cad case validate-transfer MAP.json` checks a version-1
`ConservativeTransfer` and returns its digest and bounded descriptor. It validates
units, both mesh hashes, regions, point/cell association, explicit orientation,
overlap coverage and the approved conservation-error bound. It does not run a
coupled solver or verify an imported geometry. Existing version-1 simulation
plans still reject unqualified coupled transfers.

The mesh adapter must produce and geometrically verify the overlap map against
the exact source and destination meshes. No nearest-neighbour correspondence,
face-number selection, implicit point averaging or weight renormalization is
performed by the transfer library. Missing coverage, duplicate pairs, negative
weights, invalid dimensions and conservation loss are rejected. Loss acceptance
is explicit and immutable. Arrays remain in artifacts; receipts contain identities
and integrated quantities.

## Supported scalar quantities

| Quantity                 | Field SI unit | Overlap measure                   | Conserved integral                  |
| ------------------------ | ------------- | --------------------------------- | ----------------------------------- |
| Temperature              | K             | constant thermal capacitance, J/K | capacitance-weighted temperature, J |
| Signed surface heat flux | W/m²          | area, m²                          | signed heat power, W                |
| Incident irradiance      | W/m²          | area, m²                          | incident radiant power, W           |
| Retained mass density    | kg/m³         | volume, m³                        | retained mass, kg                   |

Temperature transfer uses **constant** thermal capacitances; it is not a general
enthalpy transfer for temperature-dependent heat capacities or phase change.
Point-associated measures are declared lumped physical measures. Node counts
do not supply thermal masses. A uniform temperature remains uniform when both
meshes' declared capacitances are covered, and nonuniform transfers preserve
their weighted integral within the approved bound. Original absolute °C values
are retained and converted to K; temperature intervals use `delta_degC` or K and
cannot be confused with absolute temperatures.

Only signed heat flux permits opposing outward normals. Such a transfer reverses
the flux sign while preserving the oriented power balance. Irradiance remains
nonnegative and does not become absorbed heat without explicit optical data.
Mass transfer preserves kg and does not infer phase fraction, expansion pressure
or damage. Velocity is absent from this transfer allowlist and cannot supply a
convection coefficient.

The Rust `apply` API returns values for artifact persistence and a
`TransferReceipt`. It checks the actual integral after transfer, including signed
flux cancellation, finite numeric range and the approved tolerance. Physical
validation remains `unqualified`; library conservation tests do not qualify a
native solver, mesh correspondence or environmental claim.

## SI conversions

The quantity contract now includes thermal properties, heat capacity, area,
volume, power/energy, time, pressure, surface tension, irradiance, spectral
irradiance and radiant exposure. Spectral density `W/(m2*nm)` converts to
`W/(m2*m)` by multiplying by 10⁹; wavelength converts from nm to m independently,
so the integrated irradiance is preserved. `Wh/m2` converts to `J/m2` by 3600.
Overflow and dimensional mismatches fail validation. Original input quantities
remain available for provenance.
