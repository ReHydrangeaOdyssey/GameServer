use crate::{
    protocol::game::publicapi::{ArenaPartyCharacter, BattleCharacterStatus},
    rpc::Failure,
};
use game_logic::{
    battle::{Battle, Combatant, Mode},
    follower::{Loadout, prepare_character},
    formation::{Position, PositionEffect, correction},
    master_data::MasterData,
};
use std::collections::{BTreeMap, BTreeSet};
const TURN_LIMIT: u32 = 30;
const EMPTY_ID: u32 = u32::MAX;
pub struct Arena {
    pub master: MasterData,
    pub formations: BTreeMap<u32, Vec<PositionEffect>>,
}
impl Arena {
    pub fn prepare(
        &self,
        player: u64,
        formation: u32,
        party: &[ArenaPartyCharacter],
        side: u8,
    ) -> Result<Vec<Combatant>, Failure> {
        if !crate::rpc::ordinary_id(player) || !(1..=5).contains(&party.len()) {
            return Err(Failure::BAD_REQUEST);
        }
        let effects = self
            .formations
            .get(&formation)
            .ok_or(Failure::BAD_REQUEST)?;
        let mut characters = BTreeSet::new();
        let mut positions = BTreeSet::new();
        let mut result = Vec::new();
        for entry in party {
            if !characters.insert(entry.character_id)
                || !positions.insert(entry.position)
                || entry.follower_character_id.len() != 2
                || entry.ability_id.len() != 2
            {
                return Err(Failure::BAD_REQUEST);
            }
            let position =
                Position::new(u8::try_from(entry.position).map_err(|_| Failure::BAD_REQUEST)?)
                    .map_err(|_| Failure::BAD_REQUEST)?;
            let loadout = Loadout {
                character_id: entry.character_id,
                main_skill_id: entry.main_skill_id,
                followers: [
                    optional(entry.follower_character_id[0]),
                    optional(entry.follower_character_id[1]),
                ],
                ability_ids: entry
                    .ability_id
                    .iter()
                    .copied()
                    .filter(|id| *id != EMPTY_ID)
                    .collect(),
            };
            let character = prepare_character(&self.master, &loadout, player)
                .map_err(|_| Failure::BAD_REQUEST)?;
            let formation = correction(position, character.attribute, effects);
            result.push(Combatant {
                character,
                side,
                guild_id: 0,
                position,
                formation,
            });
        }
        Ok(result)
    }
    pub fn simulate(
        &self,
        own: Vec<Combatant>,
        enemy: Vec<Combatant>,
        seed: u64,
    ) -> Result<(), Failure> {
        // Selection uses another PRNG; combat always starts from a fresh generator with the same seed.
        let mut battle = Battle::new(
            &self.master,
            Mode::Arena,
            own.into_iter().chain(enemy).collect(),
            seed,
            Vec::new(),
            0,
        )
        .map_err(|_| Failure::BAD_REQUEST)?;
        while battle.turn_count() < TURN_LIMIT {
            if battle.step().map_err(|_| Failure::UNAVAILABLE)?.is_none() {
                break;
            }
        }
        Ok(())
    }
}
pub fn opponent(mut ids: Vec<u64>, player: u64, seed: u64) -> Result<Option<u64>, Failure> {
    ids.retain(|id| *id != player);
    ids.sort_unstable();
    if ids.iter().any(|id| !crate::rpc::ordinary_id(*id))
        || ids.windows(2).any(|pair| pair[0] == pair[1])
        || ids.len() > u32::MAX as usize
    {
        return Err(Failure::UNAVAILABLE);
    }
    game_logic::pseudorandom::shuffle(&mut ids, &mut game_logic::pseudorandom::Random::new(seed));
    Ok(ids.first().copied())
}
pub fn seed() -> Result<u64, Failure> {
    let micros = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Failure::UNAVAILABLE)?
        .as_micros();
    Ok(game_logic::pseudorandom::time_based_seed(
        u64::try_from(micros).map_err(|_| Failure::UNAVAILABLE)?,
    ))
}
pub fn snapshot(combatant: &Combatant, entry: &ArenaPartyCharacter) -> BattleCharacterStatus {
    let character = &combatant.character;
    BattleCharacterStatus {
        character_id: character.id,
        position: u32::from(combatant.position.value()),
        hp: character.hp.current_hp,
        attack: character.attack,
        defense: character.defense,
        speed_rank: character.speed as i32,
        ability_id: entry.ability_id.clone(),
        main_skill_id: entry.main_skill_id,
        max_hp: character.hp.max_hp,
    }
}
fn optional(id: u32) -> Option<u32> {
    if id == EMPTY_ID { None } else { Some(id) }
}
