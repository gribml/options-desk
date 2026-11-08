use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct UserSettings {
    pub hide_closed_positions: bool,
}
