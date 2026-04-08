use crate::TickTransaction;

#[derive(Debug, Default)]
pub struct WriteAheadLog {
    entries: Vec<TickTransaction>,
}

impl WriteAheadLog {
    pub fn append(&mut self, tx: TickTransaction) {
        self.entries.push(tx);
    }

    pub fn entries(&self) -> &[TickTransaction] {
        &self.entries
    }
}
