use graph_core::LocusId;
use graph_world::World;
use rustc_hash::FxHashMap;

#[derive(Debug, Default, Clone)]
pub struct NameMap {
    names: FxHashMap<LocusId, String>,
}

impl NameMap {
    pub fn from_world(world: &World) -> Self {
        let mut names = FxHashMap::default();
        for locus in world.loci().iter() {
            if let Some(props) = world.properties().get(locus.id)
                && let Some(name) = props.get_str("name")
            {
                names.insert(locus.id, name.to_owned());
            }
        }
        Self { names }
    }

    pub fn from_pairs<I, S>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (LocusId, S)>,
        S: Into<String>,
    {
        let names = pairs.into_iter().map(|(id, s)| (id, s.into())).collect();
        Self { names }
    }

    pub fn name(&self, locus: LocusId) -> String {
        self.names
            .get(&locus)
            .cloned()
            .unwrap_or_else(|| format!("locus_{}", locus.0))
    }

    pub fn get(&self, locus: LocusId) -> Option<&str> {
        self.names.get(&locus).map(|name| name.as_str())
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}
