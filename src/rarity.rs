use game_logic::model::Rarity;
use std::collections::BTreeMap;
/// Replace this boundary when the separate rarity source is available.
pub trait RaritySource: Send + Sync {
    fn rarity(&self, character_id: u32) -> Result<Rarity, game_logic::Error>;
}
/// Temporary explicit overrides; an absent entry never fabricates a rarity.
pub struct RarityOverrides(pub BTreeMap<u32, Rarity>);
impl RaritySource for RarityOverrides {
    fn rarity(&self, character_id: u32) -> Result<Rarity, game_logic::Error> {
        self.0
            .get(&character_id)
            .copied()
            .ok_or(game_logic::Error::MissingMasterData {
                kind: "rarity",
                id: character_id,
            })
    }
}
