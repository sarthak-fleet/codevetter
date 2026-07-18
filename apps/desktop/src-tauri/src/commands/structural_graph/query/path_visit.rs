use super::*;

#[derive(Debug)]
pub(super) struct PathVisit {
    pub(super) node_id: String,
    pub(super) cost: f64,
}

impl PathVisit {
    pub(super) fn new(node_id: String, cost: f64) -> Self {
        Self { node_id, cost }
    }
}

impl PartialEq for PathVisit {
    fn eq(&self, other: &Self) -> bool {
        self.node_id == other.node_id && self.cost == other.cost
    }
}

impl Eq for PathVisit {}

impl PartialOrd for PathVisit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PathVisit {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| other.node_id.cmp(&self.node_id))
    }
}
