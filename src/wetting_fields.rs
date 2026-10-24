//! Independent bounded reconstruction of native planar wetting contour and mass.
use crate::{Result, contracts::invalid, wetting::WettingReferenceSpec};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct FieldCheck {
    pub shape: [usize; 2],
    pub fluid_nodes: usize,
    pub droplet_area_m2: f64,
    pub contact_angle_deg: f64,
    pub circle_radius_m: f64,
    pub circle_center_m: [f64; 2],
    pub relative_radial_residual: f64,
    pub contour_points: usize,
    pub max_speed_lattice: f64,
}

pub(crate) fn assess(spec: &WettingReferenceSpec, data: &[u8]) -> Result<FieldCheck> {
    spec.validate()?;
    if data.is_empty() || data.len() > 16 * 1024 * 1024 {
        return Err(invalid("bounded complete wetting field required"));
    }
    let text = std::str::from_utf8(data).map_err(|_| invalid("native wetting CSV must be text"))?;
    let mut lines = text.lines();
    if lines.next() != Some("x_m,y_m,material,phi,u_lattice,v_lattice") {
        return Err(invalid("exact native wetting columns required"));
    }
    let n = spec.resolution as usize;
    let [nx, ny] = [5 * n / 2 + 1, 3 * n / 2 + 1];
    let dx = spec.spacing_m();
    let mut grid = vec![None; nx * ny];
    let mut amount = 0.;
    let mut correction = 0.;
    let mut max_speed: f64 = 0.;
    let mut count = 0;
    for line in lines {
        let row = line.split(',').collect::<Vec<_>>();
        if row.len() != 6 {
            return Err(invalid("complete wetting CSV row required"));
        }
        let values = row
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 2)
            .map(|(_, v)| {
                v.parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| invalid("finite wetting components required"))
            })
            .collect::<Result<Vec<_>>>()?;
        let [x, y, phi, u, v]: [f64; 5] = values
            .try_into()
            .map_err(|_| invalid("wetting components"))?;
        let ix = (x / dx).round();
        let iy = (y / dx).round();
        let material = row[2]
            .parse::<u8>()
            .map_err(|_| invalid("native material ID required"))?;
        if x < 0.
            || y < 0.
            || ix >= nx as f64
            || iy >= ny as f64
            || (x / dx - ix).abs() > 1e-8
            || (y / dx - iy).abs() > 1e-8
            || !(-0.05..=1.05).contains(&phi)
            || material
                != if iy == 0. || iy == (ny - 1) as f64 {
                    2
                } else {
                    1
                }
        {
            return Err(invalid(
                "exact native Cartesian geometry, wall material and bounded phase required",
            ));
        }
        let index = iy as usize * nx + ix as usize;
        if grid[index].replace(phi).is_some() {
            return Err(invalid("duplicate wetting lattice node"));
        }
        if material == 1 {
            // Compensated accumulation of phase area; no thresholding of phi.
            let term = (1. - phi) * dx * dx - correction;
            let next = amount + term;
            correction = (next - amount) - term;
            amount = next;
            max_speed = max_speed.max(u.hypot(v));
            count += 1;
        }
    }
    let grid = grid
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| invalid("incomplete native wetting grid"))?;
    if amount <= 0. {
        return Err(invalid("positive native phase area required"));
    }
    let mut points = Vec::new();
    for y in 1..ny - 1 {
        let row = &grid[y * nx..(y + 1) * nx];
        let mut crossings = Vec::new();
        for (x, pair) in row.windows(2).enumerate() {
            let a = pair[0] - 0.5;
            let b = pair[1] - 0.5;
            if a * b < 0. || a == 0. {
                crossings.push(if a == b {
                    x as f64
                } else {
                    x as f64 - a / (b - a)
                });
            }
        }
        if crossings.is_empty() {
            continue;
        }
        let tangent = crossings.len() == 1 && row.iter().all(|phi| *phi >= 0.5);
        if !(tangent || crossings.len() == 2)
            || crossings[0] <= 1.
            || *crossings.last().ok_or_else(|| invalid("contour absent"))? >= (nx - 2) as f64
        {
            return Err(invalid("one isolated native wetting contour required"));
        }
        if y as f64 - 0.5 >= spec.interface_width_m / dx / 2. {
            points.extend(crossings.into_iter().map(|x| [x, y as f64]));
        }
    }
    let (center, radius, residual) = circle(&points)?;
    let cosine = (0.5 - center[1]) / radius;
    if !(-1. ..1.).contains(&cosine)
        || residual > 0.05
        || (center[0] - (nx - 1) as f64 / 2.).abs() > 0.01 * radius
    {
        return Err(invalid(
            "symmetric near-circular wall-intersecting droplet required",
        ));
    }
    Ok(FieldCheck {
        shape: [nx, ny],
        fluid_nodes: count,
        droplet_area_m2: amount,
        contact_angle_deg: cosine.acos().to_degrees(),
        circle_radius_m: radius * dx,
        circle_center_m: center.map(|v| v * dx),
        relative_radial_residual: residual,
        contour_points: points.len(),
        max_speed_lattice: max_speed,
    })
}

fn circle(points: &[[f64; 2]]) -> Result<([f64; 2], f64, f64)> {
    if points.len() < 12 {
        return Err(invalid("resolved native wetting contour required"));
    }
    let mean: [f64; 2] =
        std::array::from_fn(|i| points.iter().map(|p| p[i]).sum::<f64>() / points.len() as f64);
    let mut matrix = [[0.; 4]; 3];
    for point in points {
        let [x, y] = std::array::from_fn(|i| point[i] - mean[i]);
        let row = [2. * x, 2. * y, 1.];
        let rhs = x * x + y * y;
        for i in 0..3 {
            for j in 0..3 {
                matrix[i][j] += row[i] * row[j];
            }
            matrix[i][3] += row[i] * rhs;
        }
    }
    for i in 0..3 {
        let pivot = (i..3)
            .max_by(|a, b| matrix[*a][i].abs().total_cmp(&matrix[*b][i].abs()))
            .ok_or_else(|| invalid("circle pivot required"))?;
        matrix.swap(i, pivot);
        let value = matrix[i][i];
        if value.abs() < 1e-12 {
            return Err(invalid("degenerate native contour fit"));
        }
        matrix[i] = matrix[i].map(|v| v / value);
        for k in 0..3 {
            if k != i {
                let (v, pivot) = (matrix[k][i], matrix[i]);
                for (entry, p) in matrix[k].iter_mut().zip(pivot) {
                    *entry -= v * p;
                }
            }
        }
    }
    let [cx, cy, c] = [matrix[0][3], matrix[1][3], matrix[2][3]];
    let radius = (c + cx * cx + cy * cy).sqrt();
    let center = [cx + mean[0], cy + mean[1]];
    let residual = points
        .iter()
        .map(|p| ((p[0] - center[0]).hypot(p[1] - center[1]) / radius - 1.).abs())
        .fold(0., f64::max);
    if !radius.is_finite() || radius <= 0. || !residual.is_finite() {
        return Err(invalid("positive finite native contour radius required"));
    }
    Ok((center, radius, residual))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn circle_fit_recovers_caps_and_rejects_degenerate_collinear_contours() {
        for angle in [60f64, 90., 100., 120.] {
            let radius = 24.;
            let center = [60., 0.5 - radius * angle.to_radians().cos()];
            let points = (0..=64)
                .map(|i| {
                    let alpha = (180. - angle + 2. * angle * i as f64 / 64.).to_radians();
                    [
                        center[0] + radius * alpha.cos(),
                        center[1] + radius * alpha.sin(),
                    ]
                })
                .collect::<Vec<_>>();
            let (actual, r, residual) = circle(&points).unwrap();
            assert!(
                (actual[0] - center[0]).abs() < 1e-10
                    && (actual[1] - center[1]).abs() < 1e-10
                    && (r - radius).abs() < 1e-10
                    && residual < 1e-10
            );
        }
        assert!(circle(&(0..64).map(|x| [x as f64, 1.]).collect::<Vec<_>>()).is_err());
    }
}
