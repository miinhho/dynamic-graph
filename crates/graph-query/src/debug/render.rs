use std::fmt;

use graph_core::ChangeSubject;

use super::CausalTrace;

pub(super) fn render_causal_trace(trace: &CausalTrace, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    writeln!(
        f,
        "CausalTrace for locus {} @ batch {} ({} step{})",
        trace.target.0,
        trace.batch.0,
        trace.steps.len(),
        if trace.steps.len() == 1 { "" } else { "s" },
    )?;

    if trace.steps.is_empty() {
        writeln!(f, "  (no changes found)")?;
    }

    for step in &trace.steps {
        let subject = match step.subject {
            ChangeSubject::Locus(id) => format!("Locus({})", id.0),
            ChangeSubject::Relationship(id) => format!("Rel({})", id.0),
        };
        let before = step.before.as_slice().first().copied().unwrap_or(0.0);
        let after = step.after.as_slice().first().copied().unwrap_or(0.0);
        let predecessors = render_predecessors(&step.predecessor_ids);

        writeln!(
            f,
            "  [{}] change #{:<4}  batch={:<4}  kind={:<3}  {:<12}  {:.2}→{:.2}  preds={}",
            step.depth,
            step.change_id.0,
            step.batch.0,
            step.kind.0,
            subject,
            before,
            after,
            predecessors,
        )?;
    }

    if trace.truncated {
        writeln!(
            f,
            "  *** trace truncated: predecessor(s) point into trimmed log ***"
        )?;
    }

    Ok(())
}

fn render_predecessors(predecessors: &[graph_core::ChangeId]) -> String {
    if predecessors.is_empty() {
        return String::from("[]  (root)");
    }

    let ids: Vec<String> = predecessors.iter().map(|id| format!("#{}", id.0)).collect();
    format!("[{}]", ids.join(", "))
}
