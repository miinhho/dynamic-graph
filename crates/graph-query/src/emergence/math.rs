pub(super) fn pearson_r(a: &[f64], b: &[f64]) -> Option<f64> {
    let n = a.len();
    if n != b.len() || n < 2 {
        return None;
    }
    let mean_a = mean(a);
    let mean_b = mean(b);
    let (cov, var_a, var_b) = covariance_terms(a, b, mean_a, mean_b);
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
    if !is_square_system(&a, &b) {
        return None;
    }

    for i in 0..n {
        let max_row = pivot_row(&a, i);
        if a[max_row][i].abs() < 1e-12 {
            return None;
        }
        swap_pivot(&mut a, &mut b, i, max_row);
        eliminate_below(&mut a, &mut b, i);
    }

    back_substitute(&a, &b)
}

pub(super) fn gaussian_joint_mi(x: &[Vec<f64>], y: &[f64]) -> Option<f64> {
    validate_joint_series(x, y)?;

    let y_centered = centered(y)?;
    let ss_tot: f64 = y_centered.iter().map(|value| value * value).sum();
    if ss_tot < f64::EPSILON {
        return None;
    }
    let x_centered = centered_matrix(x)?;
    let (ata, aty) = normal_equations(&x_centered, &y_centered);
    let aty_orig = aty.clone();
    let beta = solve_linear_system(ata, aty)?;
    let ss_exp: f64 = beta.iter().zip(aty_orig.iter()).map(|(b, a)| b * a).sum();
    let r2 = (ss_exp / ss_tot).clamp(0.0, 1.0);
    if r2 >= 1.0 - f64::EPSILON {
        return None;
    }
    Some(-0.5 * (1.0 - r2).ln())
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn covariance_terms(a: &[f64], b: &[f64], mean_a: f64, mean_b: f64) -> (f64, f64, f64) {
    a.iter()
        .zip(b.iter())
        .fold((0.0, 0.0, 0.0), |(cov, var_a, var_b), (&ai, &bi)| {
            let da = ai - mean_a;
            let db = bi - mean_b;
            (cov + da * db, var_a + da * da, var_b + db * db)
        })
}

fn is_square_system(a: &[Vec<f64>], b: &[f64]) -> bool {
    let n = a.len();
    n != 0 && b.len() == n && a.iter().all(|row| row.len() == n)
}

fn pivot_row(a: &[Vec<f64>], column: usize) -> usize {
    ((column + 1)..a.len()).fold(column, |max_row, row| {
        if a[row][column].abs() > a[max_row][column].abs() {
            row
        } else {
            max_row
        }
    })
}

fn swap_pivot(a: &mut [Vec<f64>], b: &mut [f64], pivot: usize, row: usize) {
    a.swap(pivot, row);
    b.swap(pivot, row);
}

fn eliminate_below(a: &mut [Vec<f64>], b: &mut [f64], pivot: usize) {
    for row_index in (pivot + 1)..a.len() {
        let (head, tail) = a.split_at_mut(row_index);
        let pivot_row = &head[pivot];
        let row = &mut tail[0];
        let factor = row[pivot] / pivot_row[pivot];
        for (value, pivot_value) in row[pivot..].iter_mut().zip(pivot_row[pivot..].iter()) {
            *value -= factor * *pivot_value;
        }
        b[row_index] -= factor * b[pivot];
    }
}

fn back_substitute(a: &[Vec<f64>], b: &[f64]) -> Option<Vec<f64>> {
    let n = a.len();
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let tail_sum: f64 = ((i + 1)..n).map(|j| a[i][j] * x[j]).sum();
        x[i] = (b[i] - tail_sum) / a[i][i];
    }
    Some(x)
}

fn validate_joint_series(x: &[Vec<f64>], y: &[f64]) -> Option<()> {
    let n = x.len();
    let m = y.len();
    (n != 0 && m >= n + 2 && x.iter().all(|series| series.len() == m)).then_some(())
}

fn centered(values: &[f64]) -> Option<Vec<f64>> {
    (!values.is_empty()).then(|| {
        let avg = mean(values);
        values.iter().map(|value| value - avg).collect()
    })
}

fn centered_matrix(x: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    x.iter().map(|series| centered(series)).collect()
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(lhs, rhs)| lhs * rhs).sum()
}

fn normal_equations(x: &[Vec<f64>], y: &[f64]) -> (Vec<Vec<f64>>, Vec<f64>) {
    let n = x.len();
    let mut ata = vec![vec![0.0; n]; n];
    let mut aty = vec![0.0; n];

    for i in 0..n {
        for j in 0..=i {
            let sum = dot(&x[i], &x[j]);
            ata[i][j] = sum;
            ata[j][i] = sum;
        }
        aty[i] = dot(&x[i], y);
    }

    (ata, aty)
}
