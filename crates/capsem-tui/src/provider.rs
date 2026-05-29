use anyhow::Result;

use crate::model::AppState;

pub trait StateProvider {
    fn load(&self) -> Result<AppState>;
}
