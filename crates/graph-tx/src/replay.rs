use graph_core::TickId;

#[derive(Debug, Clone, Copy, Default)]
pub struct ReplayCursor {
    pub next_tick: TickId,
}
