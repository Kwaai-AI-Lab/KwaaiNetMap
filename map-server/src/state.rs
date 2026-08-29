//! Shared application state

use std::sync::Arc;

use crate::cache::NodeCache;
use crate::reachability::Reachability;

pub struct AppState {
    pub cache: Arc<NodeCache>,
    /// `TOTAL_BLOCKS` override. `None` measures coverage against the largest
    /// model the DHT actually registers, which is right far more often.
    pub total_blocks: Option<usize>,
    pub reachability: Reachability,
}

pub type SharedState = Arc<AppState>;
