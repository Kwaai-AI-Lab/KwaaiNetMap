//! Shared application state

use std::sync::Arc;

use crate::cache::NodeCache;

pub struct AppState {
    pub cache: Arc<NodeCache>,
    /// Total model blocks (used for coverage calculation)
    pub total_blocks: usize,
}

pub type SharedState = Arc<AppState>;
