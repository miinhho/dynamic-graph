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

    let mut sorted: Vec<&DropResult> = result.drops.iter().collect();
    sorted.sort_by(|a, b| {
        b.psi_pair_top3_delta
            .abs()
            .partial_cmp(&a.psi_pair_top3_delta.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if !sorted.is_empty() {
        let _ = writeln!(&mut out, "\n### Per-drop effect, top 10 by |Δ Ψ_pair_top3|");
        let _ = writeln!(
            &mut out,
            "\n| dropped | Ψ_corr | Ψ_pair_top3 | Δ Ψ_corr | Δ Ψ_pair_top3 |"
        );
        let _ = writeln!(&mut out, "|---|---|---|---|---|");
        for drop in sorted.iter().take(10) {
            let _ = writeln!(
                &mut out,
                "| {:?} | {:+.4} | {:+.4} | {:+.4} | {:+.4} |",
                drop.dropped,
                drop.psi_corrected,
                drop.psi_pair_top3,
                drop.psi_corrected_delta,
                drop.psi_pair_top3_delta,
            );
        }
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
    if rows.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n### {heading}");
    let _ = writeln!(out, "\n| entity | Ψ | I_self | Σ I_components | n |");
    let _ = writeln!(out, "|---|---|---|---|---|");
    for entry in rows.iter().take(10) {
        let _ = writeln!(
            out,
            "| {:?} | {:+.4} | {:.4} | {:.4} | {} |",
            entry.entity,
            entry.psi.psi,
            entry.psi.i_self,
            entry.psi.i_sum_components,
            entry.psi.n_samples,
        );
    }
}

fn render_synergy_section(out: &mut String, heading: &str, rows: &[EmergenceSynergyEntry]) {
    if rows.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n### {heading}");
    let _ = writeln!(
        out,
        "\n| entity | Ψ_corr | Ψ_naive | I_self | I_joint | Σ I_i | n | comp |",
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|");
    for entry in rows.iter().take(10) {
        let _ = writeln!(
            out,
            "| {:?} | {:+.4} | {:+.4} | {:.4} | {:.4} | {:.4} | {} | {} |",
            entry.entity,
            entry.psi.psi_corrected,
            entry.psi.psi_naive,
            entry.psi.i_self,
            entry.psi.i_joint_components,
            entry.psi.i_sum_components,
            entry.psi.n_samples,
            entry.psi.n_components,
        );
    }
}

fn render_top_pair_section(out: &mut String, report: &EmergenceSynergyReport) {
    let rows: Vec<(&EmergenceSynergyEntry, &SynergyPair)> = report
        .emergent
        .iter()
        .chain(report.spurious.iter())
        .filter_map(|entry| entry.psi.top_pairs.first().map(|pair| (entry, pair)))
        .take(10)
        .collect();
    if rows.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n### Top synergistic pair per entity, top 10");
    let _ = writeln!(
        out,
        "\n| entity | pair (a, b) | synergy | joint_mi | redundancy | mi_a | mi_b |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|");
    for (entry, pair) in rows {
        let _ = writeln!(
            out,
            "| {:?} | {:?}, {:?} | {:+.4} | {:.4} | {:.4} | {:.4} | {:.4} |",
            entry.entity,
            pair.a,
            pair.b,
            pair.synergy,
            pair.joint_mi,
            pair.redundancy,
            pair.mi_a,
            pair.mi_b,
        );
    }
}

fn render_pair_grain_section(out: &mut String, report: &EmergenceSynergyReport) {
    let rows: Vec<&EmergenceSynergyEntry> = report
        .emergent
        .iter()
        .chain(report.spurious.iter())
        .filter(|entry| entry.psi.n_pairs_evaluated > 0)
        .take(10)
        .collect();
    if rows.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n### Pair-grain emergence (H5), top 10 measured");
    let _ = writeln!(
        out,
        "\n| entity | Ψ_pair_top3 | Σ synergy | Σ redundancy | mean synergy | n_pairs |",
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|");
    for entry in rows {
        let _ = writeln!(
            out,
            "| {:?} | {:+.4} | {:+.4} | {:.4} | {:+.4} | {} |",
            entry.entity,
            entry.psi.psi_pair_top3,
            entry.psi.total_pair_synergy,
            entry.psi.total_pair_redundancy,
            entry.psi.mean_pair_synergy,
            entry.psi.n_pairs_evaluated,
        );
    }
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
