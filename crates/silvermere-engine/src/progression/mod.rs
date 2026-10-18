//! Experience, levels and the top list (T-039):
//!
//! - [`experience`]: `add_experience`, the gain and the training cap;
//! - [`train`]: `train_level`, TRAIN's level-up;
//! - [`topten`]: the experience top list on `World::topten`.
//!
//! The arithmetic (experience needed, fee, gains) is `silvermere_rules::progression`.

pub mod experience;
pub mod topten;
pub mod train;

pub use experience::{ExpOutcome, add_experience};
pub use topten::TopTen;
pub use train::train_level;

use silvermere_types::ids::SessionId;

use crate::event_loop::{EngineOutput, GameEvent};
use crate::world::World;

impl World {
    /// A character reached `level` by training: the `levelup` metric (T-059) and the
    /// admin `level` event (T-060).
    pub fn level_gained(&mut self, id: SessionId, level: u16) {
        crate::metrics::levelup();
        self.outbox
            .push(EngineOutput::Event(GameEvent::Level { id, level }));
    }
}
