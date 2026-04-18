use super::{
    DropResult, EmergenceEntry, EmergenceReport, EmergenceSynergyEntry, EmergenceSynergyReport,
    LeaveOneOutResult, SynergyPair, UnmeasuredEntry, UnmeasuredReason,
};
use std::fmt::Write;

pub(super) fn render_leave_one_out_markdown(result: &LeaveOneOutResult) -> String {
    let mut out = String::new();
    let _ = writeln!(&mut out, "## Leave-one-out — {:?}\n", result.entity);
    let _ = writeln!(
        &mut out,
        "- baseline: Ψ_corrected = {:+.4}, Ψ_pair_top3 = {:+.4}",
        result.baseline.psi_corrected, result.baseline.psi_pair_top3
    );
    let _ = writeln!(
        &mut out,
        "- components: **{}**, pairs evaluated: **{}**",
        result.baseline.n_components, result.baseline.n_pairs_evaluated
    );
    let _ = writeln!(
        &mut out,
        "- sign flips (Ψ_corrected): **{}** / {}",
        result.sign_flips_corrected(),
        result.drops.len()
    );
    let _ = writeln!(
        &mut out,
        "- sign flips (Ψ_pair_top3): **{}** / {}",
        result.sign_flips_pair_top3(),
        result.drops.len()
    );

    let rows = sorted_drop_rows(result);
    if !rows.is_empty() {
        render_table(
            &mut out,
            "Per-drop effect, top 10 by |Δ Ψ_pair_top3|",
            &[
                "dropped",
                "Ψ_corr",
                "Ψ_pair_top3",
                "Δ Ψ_corr",
                "Δ Ψ_pair_top3",
            ],
            &rows,
        );
    }
    out
}

pub(super) fn render_emergence_report_markdown(report: &EmergenceReport) -> String {
    let mut out = String::new();
    let _ = writeln!(&mut out, "## Emergence report\n");
    let _ = writeln!(&mut out, "- entities total: **{}**", report.n_entities);
    let _ = writeln!(
        &mut out,
        "- measured: **{}** (emergent {}, spurious {})",
        report.n_measured(),
        report.emergent.len(),
        report.spurious.len()
    );
    let _ = writeln!(&mut out, "- unmeasured: **{}**", report.unmeasured.len());
    if let Some(fraction) = report.emergent_fraction() {
        let _ = writeln!(
            &mut out,
            "- emergent fraction: **{:.1}%**",
            fraction * 100.0
        );
    }

    render_scalar_section(&mut out, "Emergent (Ψ > 0), top 10", &report.emergent);
    render_scalar_section(&mut out, "Spurious (Ψ ≤ 0), top 10", &report.spurious);
    render_unmeasured_breakdown(&mut out, &report.unmeasured);
    out
}

pub(super) fn render_synergy_report_markdown(report: &EmergenceSynergyReport) -> String {
    let mut out = String::new();
    let _ = writeln!(&mut out, "## Emergence report (synergy-corrected)\n");
    let _ = writeln!(&mut out, "- entities total: **{}**", report.n_entities);
    let _ = writeln!(
        &mut out,
        "- measured: **{}** (emergent {}, spurious {})",
        report.n_measured(),
        report.emergent.len(),
        report.spurious.len()
    );
    let _ = writeln!(&mut out, "- unmeasured: **{}**", report.unmeasured.len());
    if let Some(fraction) = report.emergent_fraction() {
        let _ = writeln!(
            &mut out,
            "- emergent fraction (Ψ_corrected > 0): **{:.1}%**",
            fraction * 100.0
        );
    }

    render_synergy_section(
        &mut out,
        "Emergent (Ψ_corrected > 0), top 10",
        &report.emergent,
    );
    render_synergy_section(
        &mut out,
        "Spurious (Ψ_corrected ≤ 0), top 10",
        &report.spurious,
    );
    render_top_pair_section(&mut out, report);
    render_pair_grain_section(&mut out, report);
    render_unmeasured_breakdown(&mut out, &report.unmeasured);
    out
}

fn render_scalar_section(out: &mut String, heading: &str, rows: &[EmergenceEntry]) {
    render_table(
        out,
        heading,
        &["entity", "Ψ", "I_self", "Σ I_components", "n"],
        &scalar_rows(rows),
    );
}

fn render_synergy_section(out: &mut String, heading: &str, rows: &[EmergenceSynergyEntry]) {
    render_table(
        out,
        heading,
        &[
            "entity", "Ψ_corr", "Ψ_naive", "I_self", "I_joint", "Σ I_i", "n", "comp",
        ],
        &synergy_rows(rows),
    );
}

fn render_top_pair_section(out: &mut String, report: &EmergenceSynergyReport) {
    render_table(
        out,
        "Top synergistic pair per entity, top 10",
        &[
            "entity",
            "pair (a, b)",
            "synergy",
            "joint_mi",
            "redundancy",
            "mi_a",
            "mi_b",
        ],
        &top_pair_rows(report),
    );
}

fn render_pair_grain_section(out: &mut String, report: &EmergenceSynergyReport) {
    render_table(
        out,
        "Pair-grain emergence (H5), top 10 measured",
        &[
            "entity",
            "Ψ_pair_top3",
            "Σ synergy",
            "Σ redundancy",
            "mean synergy",
            "n_pairs",
        ],
        &pair_grain_rows(report),
    );
}

fn render_unmeasured_breakdown(out: &mut String, rows: &[UnmeasuredEntry]) {
    if rows.is_empty() {
        return;
    }
    let counts = count_unmeasured(rows);
    let _ = writeln!(out, "\n### Unmeasured breakdown");
    let _ = writeln!(out, "- dormant: {}", counts.dormant);
    let _ = writeln!(out, "- insufficient stable window: {}", counts.short_window);
    let _ = writeln!(
        out,
        "- no component history: {}",
        counts.no_component_history
    );
}

struct UnmeasuredCounts {
    dormant: usize,
    short_window: usize,
    no_component_history: usize,
}

fn count_unmeasured(rows: &[UnmeasuredEntry]) -> UnmeasuredCounts {
    rows.iter().fold(
        UnmeasuredCounts {
            dormant: 0,
            short_window: 0,
            no_component_history: 0,
        },
        |mut counts, entry| {
            match entry.reason {
                UnmeasuredReason::Dormant => counts.dormant += 1,
                UnmeasuredReason::InsufficientStableWindow { .. } => counts.short_window += 1,
                UnmeasuredReason::NoComponentHistory => counts.no_component_history += 1,
            }
            counts
        },
    )
}

fn render_table(out: &mut String, heading: &str, headers: &[&str], rows: &[Vec<String>]) {
    if rows.is_empty() {
        return;
    }

    let _ = writeln!(out, "\n### {heading}");
    let _ = writeln!(out, "\n| {} |", headers.join(" | "));
    let _ = writeln!(out, "|{}|", vec!["---"; headers.len()].join("|"));
    for row in rows {
        let _ = writeln!(out, "| {} |", row.join(" | "));
    }
}

fn sorted_drop_rows(result: &LeaveOneOutResult) -> Vec<Vec<String>> {
    let mut sorted: Vec<&DropResult> = result.drops.iter().collect();
    sorted.sort_by(|a, b| {
        b.psi_pair_top3_delta
            .abs()
            .partial_cmp(&a.psi_pair_top3_delta.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    sorted
        .into_iter()
        .take(10)
        .map(|drop| {
            vec![
                format!("{:?}", drop.dropped),
                format!("{:+.4}", drop.psi_corrected),
                format!("{:+.4}", drop.psi_pair_top3),
                format!("{:+.4}", drop.psi_corrected_delta),
                format!("{:+.4}", drop.psi_pair_top3_delta),
            ]
        })
        .collect()
}

fn scalar_rows(rows: &[EmergenceEntry]) -> Vec<Vec<String>> {
    rows.iter()
        .take(10)
        .map(|entry| {
            vec![
                format!("{:?}", entry.entity),
                format!("{:+.4}", entry.psi.psi),
                format!("{:.4}", entry.psi.i_self),
                format!("{:.4}", entry.psi.i_sum_components),
                entry.psi.n_samples.to_string(),
            ]
        })
        .collect()
}

fn synergy_rows(rows: &[EmergenceSynergyEntry]) -> Vec<Vec<String>> {
    rows.iter()
        .take(10)
        .map(|entry| {
            vec![
                format!("{:?}", entry.entity),
                format!("{:+.4}", entry.psi.psi_corrected),
                format!("{:+.4}", entry.psi.psi_naive),
                format!("{:.4}", entry.psi.i_self),
                format!("{:.4}", entry.psi.i_joint_components),
                format!("{:.4}", entry.psi.i_sum_components),
                entry.psi.n_samples.to_string(),
                entry.psi.n_components.to_string(),
            ]
        })
        .collect()
}

fn top_pair_rows(report: &EmergenceSynergyReport) -> Vec<Vec<String>> {
    report
        .emergent
        .iter()
        .chain(report.spurious.iter())
        .filter_map(|entry| {
            entry
                .psi
                .top_pairs
                .first()
                .map(|pair| top_pair_row(entry, pair))
        })
        .take(10)
        .collect()
}

fn top_pair_row(entry: &EmergenceSynergyEntry, pair: &SynergyPair) -> Vec<String> {
    vec![
        format!("{:?}", entry.entity),
        format!("{:?}, {:?}", pair.a, pair.b),
        format!("{:+.4}", pair.synergy),
        format!("{:.4}", pair.joint_mi),
        format!("{:.4}", pair.redundancy),
        format!("{:.4}", pair.mi_a),
        format!("{:.4}", pair.mi_b),
    ]
}

fn pair_grain_rows(report: &EmergenceSynergyReport) -> Vec<Vec<String>> {
    report
        .emergent
        .iter()
        .chain(report.spurious.iter())
        .filter(|entry| entry.psi.n_pairs_evaluated > 0)
        .take(10)
        .map(|entry| {
            vec![
                format!("{:?}", entry.entity),
                format!("{:+.4}", entry.psi.psi_pair_top3),
                format!("{:+.4}", entry.psi.total_pair_synergy),
                format!("{:.4}", entry.psi.total_pair_redundancy),
                format!("{:+.4}", entry.psi.mean_pair_synergy),
                entry.psi.n_pairs_evaluated.to_string(),
            ]
        })
        .collect()
}
