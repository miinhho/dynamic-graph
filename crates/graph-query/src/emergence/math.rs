pub(super) fn pearson_r(a: &[f64], b: &[f64]) -> Option<f64> {
    let n = a.len();
    if n != b.len() || n < 2 {
        return None;
    }
    let mean_a = a.iter().sum::<f64>() / n as f64;
    let mean_b = b.iter().sum::<f64>() / n as f64;

    let (cov, var_a, var_b) =
        a.iter()
            .zip(b.iter())
            .fold((0.0, 0.0, 0.0), |(cov, var_a, var_b), (&ai, &bi)| {
                let da = ai - mean_a;
                let db = bi - mean_b;
                (cov + da * db, var_a + da * da, var_b + db * db)
            });
    if var_a < f64::EPSILON || var_b < f64::EPSILON {
        return None;
    }
    Some(cov / (var_a * var_b).sqrt())
}

pub(super) fn gaussian_mi_from_series(a: &[f64], b: &[f64]) -> Option<f64> {
    let r = pearson_r(a, b)?;
    let r2 = r * r;
    if r2 >= 1.0 - f64::EPSILON {
        return None;
    }
    Some(-0.5 * (1.0 - r2).ln())
}

pub(super) fn solve_linear_system(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = a.len();
    if n == 0 || b.len() != n || a.iter().any(|row| row.len() != n) {
        return None;
    }

    for i in 0..n {
        let mut max_row = i;
        for k in (i + 1)..n {
            if a[k][i].abs() > a[max_row][i].abs() {
                max_row = k;
            }
        }
        if a[max_row][i].abs() < 1e-12 {
            return None;
        }
        a.swap(i, max_row);
        b.swap(i, max_row);

        for k in (i + 1)..n {
            let (head, tail) = a.split_at_mut(k);
            let pivot_row = &head[i];
            let row = &mut tail[0];
            let factor = row[i] / pivot_row[i];
            for (a_kj, a_ij) in row[i..n].iter_mut().zip(pivot_row[i..n].iter()) {
                *a_kj -= factor * *a_ij;
            }
            b[k] -= factor * b[i];
        }
    }

    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut sum = b[i];
        for j in (i + 1)..n {
            sum -= a[i][j] * x[j];
        }
        x[i] = sum / a[i][i];
    }
    Some(x)
}

pub(super) fn gaussian_joint_mi(x: &[Vec<f64>], y: &[f64]) -> Option<f64> {
    let n = x.len();
    if n == 0 {
        return None;
    }
    let m = y.len();
    if m < n + 2 || x.iter().any(|series| series.len() != m) {
        return None;
    }

    let y_mean = y.iter().sum::<f64>() / m as f64;
    let y_centered: Vec<f64> = y.iter().map(|value| value - y_mean).collect();
    let ss_tot: f64 = y_centered.iter().map(|value| value * value).sum();
    if ss_tot < f64::EPSILON {
        return None;
    }

    let x_centered: Vec<Vec<f64>> = x
        .iter()
        .map(|series| {
            let mean = series.iter().sum::<f64>() / m as f64;
            series.iter().map(|value| value - mean).collect()
        })
        .collect();

    let mut ata = vec![vec![0.0; n]; n];
    let mut aty = vec![0.0; n];
    for i in 0..n {
        for j in 0..=i {
            let sum: f64 = x_centered[i]
                .iter()
                .zip(x_centered[j].iter())
                .map(|(a, b)| a * b)
                .sum();
            ata[i][j] = sum;
            ata[j][i] = sum;
        }
        aty[i] = x_centered[i]
            .iter()
            .zip(y_centered.iter())
            .map(|(a, b)| a * b)
            .sum();
    }

    let aty_orig = aty.clone();
    let beta = solve_linear_system(ata, aty)?;
    let ss_exp: f64 = beta.iter().zip(aty_orig.iter()).map(|(b, a)| b * a).sum();
    let r2 = (ss_exp / ss_tot).clamp(0.0, 1.0);
    if r2 >= 1.0 - f64::EPSILON {
        return None;
    }
    Some(-0.5 * (1.0 - r2).ln())
}
