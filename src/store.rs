use std::collections::HashMap;

use leptos::prelude::*;

use crate::models::market::{LatestBar, OptionMetaEntry};

/// Global market data cache shared across all pages.
/// Provided at app root; consumed via use_context::<MarketStore>().
#[derive(Clone, Copy)]
pub struct MarketStore {
    pub quotes: RwSignal<HashMap<String, LatestBar>>,
    pub option_meta: RwSignal<HashMap<String, Vec<OptionMetaEntry>>>,
}

impl MarketStore {
    pub fn new() -> Self {
        Self {
            quotes: RwSignal::new(HashMap::new()),
            option_meta: RwSignal::new(HashMap::new()),
        }
    }
}
