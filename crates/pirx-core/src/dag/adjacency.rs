//! DAG adjacency maps — held separately from node data for cache locality.

use serde::{Deserialize, Serialize};
use slotmap::SecondaryMap;
use smallvec::SmallVec;

use super::kind::OpKey;

/// DAG adjacency — held separately from node data for cache locality.
///
/// Node data is hot during scheduling; adjacency is hot during ready-set
/// computation. Separate `SecondaryMap`s let them occupy different cache lines.
#[derive(Debug, Serialize, Deserialize)]
pub struct DagAdjacency {
    /// Successor keys for each node.
    ///
    /// `SmallVec<[OpKey; 4]>` — 32 bytes inline, no heap allocation for the
    /// common 1-4 successors case.
    pub successors: SecondaryMap<OpKey, SmallVec<[OpKey; 4]>>,
    /// Predecessor keys for each node. Used by [`Dag::activate_ops`] to
    /// recompute effective predecessor counts after activation.
    pub predecessors: SecondaryMap<OpKey, SmallVec<[OpKey; 4]>>,
    /// Number of predecessors not yet completed. Decremented by
    /// [`Dag::release_successors`]. Reaches zero when the node is ready.
    pub predecessor_count: SecondaryMap<OpKey, u32>,
}
