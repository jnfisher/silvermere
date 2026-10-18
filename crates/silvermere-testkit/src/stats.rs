//! The statistics of the M2 A/B comparisons: chi-square goodness of fit against a closed-form distribution, chi-square
//! homogeneity of two histograms, and the two-proportion z-test.
//!
//! Standard library only, so `silvermere-engine/tests/m2_fight.rs` can include this file by
//! path without a dependency on the testkit. `tools/differential/stats.py` is the same code
//! in Python (same pooling rule, same incomplete-gamma series and continued fraction), so
//! both sides compute the same statistic on the same counts; its tests pin the numbers
//! below.

/// One test's result: the statistic, its degrees of freedom (0 for a z-test) and the p-value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Test {
    pub statistic: f64,
    pub df: u32,
    pub p_value: f64,
}

impl Test {
    /// The comparison holds: `p_value > alpha` (the plan uses `alpha = 0.01`).
    pub fn passes(&self, alpha: f64) -> bool {
        self.p_value > alpha
    }
}

/// Bins whose expected count is below this are merged with their neighbour.
pub const MIN_EXPECTED: f64 = 5.0;

/// `ln Γ(x)` for `x > 0` (Lanczos, g = 7, nine coefficients).
pub fn ln_gamma(x: f64) -> f64 {
    const G: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        // reflection
        let pi = std::f64::consts::PI;
        return (pi / (pi * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = G[0];
    let t = x + 7.5;
    for (i, g) in G.iter().enumerate().skip(1) {
        a += g / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// The regularised upper incomplete gamma function `Q(a, x)`: the series below `a + 1`,
/// the Lentz continued fraction above (Numerical Recipes `gammq`).
pub fn gamma_q(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 1.0;
    }
    let gln = ln_gamma(a);
    if x < a + 1.0 {
        let mut ap = a;
        let mut sum = 1.0 / a;
        let mut del = sum;
        for _ in 0..1000 {
            ap += 1.0;
            del *= x / ap;
            sum += del;
            if del.abs() < sum.abs() * 1e-15 {
                break;
            }
        }
        let p = sum * (-x + a * x.ln() - gln).exp();
        (1.0 - p).max(0.0)
    } else {
        const TINY: f64 = 1e-300;
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / TINY;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..1000 {
            let an = -(i as f64) * (i as f64 - a);
            b += 2.0;
            d = an * d + b;
            if d.abs() < TINY {
                d = TINY;
            }
            c = b + an / c;
            if c.abs() < TINY {
                c = TINY;
            }
            d = 1.0 / d;
            let del = d * c;
            h *= del;
            if (del - 1.0).abs() < 1e-15 {
                break;
            }
        }
        (-x + a * x.ln() - gln).exp() * h
    }
}

/// `P(X > stat)` for a chi-square variable with `df` degrees of freedom; 1 when `df` is 0.
pub fn chi_square_sf(stat: f64, df: u32) -> f64 {
    if df == 0 {
        return 1.0;
    }
    gamma_q(f64::from(df) / 2.0, stat / 2.0)
}

/// Two-sided `P(|Z| > |z|)` for a standard normal: `erfc(|z|/√2)`, with
/// `erfc(t) = Q(1/2, t²)`.
pub fn normal_two_sided(z: f64) -> f64 {
    let t = z.abs() / std::f64::consts::SQRT_2;
    gamma_q(0.5, t * t)
}

/// Merges adjacent bins, left to right, until each merged bin's `expected` reaches
/// [`MIN_EXPECTED`]; a short last run joins the bin before it. Returns the merged
/// `(observed, expected)` pairs.
pub fn pool(observed: &[f64], expected: &[f64]) -> Vec<(f64, f64)> {
    assert_eq!(observed.len(), expected.len());
    let mut out: Vec<(f64, f64)> = Vec::new();
    let (mut o, mut e) = (0.0, 0.0);
    for (&ob, &ex) in observed.iter().zip(expected) {
        o += ob;
        e += ex;
        if e >= MIN_EXPECTED {
            out.push((o, e));
            o = 0.0;
            e = 0.0;
        }
    }
    if o > 0.0 || e > 0.0 {
        match out.last_mut() {
            Some(last) => {
                last.0 += o;
                last.1 += e;
            }
            None => out.push((o, e)),
        }
    }
    out
}

/// Pearson's goodness of fit of `observed` counts against the probabilities `probs` (which
/// sum to 1), after [`pool`]. `df` is the number of pooled bins less one.
pub fn chi_square_gof(observed: &[u64], probs: &[f64]) -> Test {
    let n: u64 = observed.iter().sum();
    let obs: Vec<f64> = observed.iter().map(|&c| c as f64).collect();
    let exp: Vec<f64> = probs.iter().map(|&p| p * n as f64).collect();
    let bins = pool(&obs, &exp);
    let statistic = bins
        .iter()
        .filter(|(_, e)| *e > 0.0)
        .map(|(o, e)| (o - e) * (o - e) / e)
        .sum();
    let df = u32::try_from(bins.len().saturating_sub(1)).unwrap_or(u32::MAX);
    Test {
        statistic,
        df,
        p_value: chi_square_sf(statistic, df),
    }
}

/// Chi-square homogeneity of two histograms over the same bins (a 2 x k table): do `a` and
/// `b` come from one distribution? Bins are pooled on the smaller sample's expected count.
pub fn chi_square_homogeneity(a: &[u64], b: &[u64]) -> Test {
    assert_eq!(a.len(), b.len(), "histograms over the same bins");
    let na: u64 = a.iter().sum();
    let nb: u64 = b.iter().sum();
    let total = (na + nb) as f64;
    if na == 0 || nb == 0 {
        return Test {
            statistic: 0.0,
            df: 0,
            p_value: 1.0,
        };
    }
    // pool on the column totals so both rows share the same bins
    let col: Vec<f64> = a.iter().zip(b).map(|(&x, &y)| (x + y) as f64).collect();
    let small = na.min(nb) as f64 / total;
    let weight: Vec<f64> = col.iter().map(|c| c * small).collect();
    let mut groups: Vec<(u64, u64)> = Vec::new();
    let (mut ga, mut gb, mut gw) = (0u64, 0u64, 0.0);
    for i in 0..a.len() {
        ga += a[i];
        gb += b[i];
        gw += weight[i];
        if gw >= MIN_EXPECTED {
            groups.push((ga, gb));
            ga = 0;
            gb = 0;
            gw = 0.0;
        }
    }
    if ga + gb > 0 {
        match groups.last_mut() {
            Some(last) => {
                last.0 += ga;
                last.1 += gb;
            }
            None => groups.push((ga, gb)),
        }
    }
    let mut statistic = 0.0;
    for &(x, y) in &groups {
        let c = (x + y) as f64;
        let ea = c * na as f64 / total;
        let eb = c * nb as f64 / total;
        if ea > 0.0 {
            statistic += (x as f64 - ea).powi(2) / ea;
        }
        if eb > 0.0 {
            statistic += (y as f64 - eb).powi(2) / eb;
        }
    }
    let df = u32::try_from(groups.len().saturating_sub(1)).unwrap_or(u32::MAX);
    Test {
        statistic,
        df,
        p_value: chi_square_sf(statistic, df),
    }
}

/// The pooled two-proportion z-test of `x1/n1` against `x2/n2`, two-sided.
pub fn two_proportion_z(x1: u64, n1: u64, x2: u64, n2: u64) -> Test {
    let (n1f, n2f) = (n1 as f64, n2 as f64);
    let p = (x1 + x2) as f64 / (n1f + n2f);
    let se = (p * (1.0 - p) * (1.0 / n1f + 1.0 / n2f)).sqrt();
    let z = if se > 0.0 {
        (x1 as f64 / n1f - x2 as f64 / n2f) / se
    } else {
        0.0
    };
    Test {
        statistic: z,
        df: 0,
        p_value: normal_two_sided(z),
    }
}

/// The one-sample z-test of `x` successes in `n` against the rate `p`, two-sided.
pub fn binomial_z(x: u64, n: u64, p: f64) -> Test {
    let nf = n as f64;
    let se = (nf * p * (1.0 - p)).sqrt();
    let z = if se > 0.0 {
        (x as f64 - nf * p) / se
    } else {
        0.0
    };
    let p_value = if se > 0.0 {
        normal_two_sided(z)
    } else if (x as f64 - nf * p).abs() < 0.5 {
        1.0
    } else {
        0.0
    };
    Test {
        statistic: z,
        df: 0,
        p_value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn chi_square_table_values() {
        // the 5% and 1% points of the chi-square table
        assert!(close(chi_square_sf(3.841_459, 1), 0.05, 1e-6));
        assert!(close(chi_square_sf(9.210_340, 2), 0.01, 1e-6));
        assert!(close(chi_square_sf(23.209_25, 10), 0.01, 1e-6));
        assert!(close(chi_square_sf(0.0, 4), 1.0, 1e-12));
        assert_eq!(chi_square_sf(5.0, 0), 1.0);
    }

    #[test]
    fn normal_table_values() {
        assert!(close(normal_two_sided(1.959_964), 0.05, 1e-6));
        assert!(close(normal_two_sided(-2.575_829), 0.01, 1e-6));
        assert!(close(normal_two_sided(0.0), 1.0, 1e-12));
    }

    #[test]
    fn ln_gamma_values() {
        assert!(close(ln_gamma(1.0), 0.0, 1e-12));
        assert!(close(ln_gamma(5.0), 24f64.ln(), 1e-12));
        assert!(close(
            ln_gamma(0.5),
            std::f64::consts::PI.sqrt().ln(),
            1e-12
        ));
    }

    #[test]
    fn pooling_merges_small_bins() {
        let bins = pool(&[1.0, 1.0, 10.0, 2.0], &[2.0, 4.0, 10.0, 1.0]);
        assert_eq!(bins, vec![(2.0, 6.0), (12.0, 11.0)]);
    }

    #[test]
    fn goodness_of_fit_and_homogeneity() {
        let fair = chi_square_gof(&[100, 100, 100, 100], &[0.25; 4]);
        assert_eq!(fair.df, 3);
        assert!(close(fair.statistic, 0.0, 1e-12) && fair.passes(0.01));
        let skew = chi_square_gof(&[160, 80, 80, 80], &[0.25; 4]);
        assert!(close(skew.statistic, 48.0, 1e-9) && !skew.passes(0.01));
        let same = chi_square_homogeneity(&[50, 30, 20], &[100, 60, 40]);
        assert!(close(same.statistic, 0.0, 1e-12) && same.df == 2);
        let diff = chi_square_homogeneity(&[90, 5, 5], &[30, 35, 35]);
        assert!(!diff.passes(0.01));
    }

    #[test]
    fn z_tests() {
        let t = two_proportion_z(620, 1000, 600, 1000);
        assert!(close(t.statistic, 0.916_891, 1e-5), "{t:?}");
        assert!(t.passes(0.01));
        let b = binomial_z(900, 1000, 0.9);
        assert!(close(b.statistic, 0.0, 1e-12) && b.passes(0.01));
        assert!(!binomial_z(50, 1000, 0.9).passes(0.01));
    }
}
