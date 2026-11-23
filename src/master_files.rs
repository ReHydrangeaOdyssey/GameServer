use crate::{
    arena::Arena, master_conversion as convert, protocol::game::model as wire,
    rarity::RarityOverrides, transport::Error,
};
use game_logic::{
    formation::{Correction, Position, PositionEffect},
    master_data::MasterData,
    model as core,
};
use std::collections::BTreeMap;
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MasterFiles {
    pub characters: Vec<String>,
    pub skills: Vec<String>,
    pub abilities: Vec<String>,
    pub tactics: Vec<String>,
    pub rarity_overrides: BTreeMap<u32, u32>,
    pub formations: Vec<Formation>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Formation {
    pub id: u32,
    pub effects: Vec<Effect>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    pub position: u8,
    pub condition: u32,
    pub attack: f32,
    pub defense: f32,
    pub speed: f32,
    pub skill_activation_rate: f32,
}
impl MasterFiles {
    pub async fn load(self) -> Result<Arena, Error> {
        let mut rarity = BTreeMap::new();
        for (id, value) in self.rarity_overrides {
            rarity.insert(id, core::Rarity::try_from(value)?);
        }
        let rarity = RarityOverrides(rarity);
        let mut data = core::ProcessedMasterData::default();
        let mut composites = Vec::new();
        for path in self.skills {
            let skill = convert::skill(read::<wire::SkillMasterData>(&path).await?)?;
            data.skills.push(skill.main_effect.effect.clone());
            composites.push(skill);
        }
        for path in self.abilities {
            data.abilities.push(convert::ability(read(&path).await?)?);
        }
        for path in self.tactics {
            data.tactics.push(convert::tactics(read(&path).await?)?);
        }
        for path in self.characters {
            data.characters
                .push(convert::character(read(&path).await?, &rarity)?);
        }
        if data.characters.is_empty() {
            return Err("character master is empty".into());
        }
        let master = MasterData::new(data)?.with_composite_skills(composites)?;
        let mut formations = BTreeMap::new();
        for entry in self.formations {
            let mut effects = Vec::new();
            for e in entry.effects {
                if [e.attack, e.defense, e.speed, e.skill_activation_rate]
                    .iter()
                    .any(|v| !v.is_finite())
                    || e.speed.fract() != 0.0
                {
                    return Err("invalid formation correction".into());
                }
                effects.push(PositionEffect {
                    position: Position::new(e.position)?,
                    condition: core::FormationConditionID::try_from(e.condition)?,
                    correction: Correction {
                        attack: e.attack,
                        defense: e.defense,
                        speed: e.speed,
                        skill_activation_rate: e.skill_activation_rate,
                    },
                });
            }
            if formations.insert(entry.id, effects).is_some() {
                return Err("duplicate formation".into());
            }
        }
        if formations.is_empty() {
            return Err("formation master is empty".into());
        }
        Ok(Arena { master, formations })
    }
}
async fn read<M: prost::Message + Default>(path: &str) -> Result<M, Error> {
    let bytes = tokio::fs::read(path).await?;
    Ok(M::decode(bytes.as_slice())?)
}
