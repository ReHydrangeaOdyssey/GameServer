use crate::protocol::game::model as wire;
use game_logic::{
    Error,
    composite_skill::{CompositeSkill, SkillEffectComponent},
    model as core,
};
pub fn enumeration<T: TryFrom<u32, Error = Error>>(value: i32) -> Result<T, Error> {
    T::try_from(u32::try_from(value).map_err(|_| Error::InvalidValue("negative enum"))?)
}
fn required<T>(value: Option<T>) -> Result<T, Error> {
    value.ok_or(Error::InvalidValue("missing master field"))
}
pub fn character(
    value: wire::CharacterMasterData,
    rarity: &dyn crate::rarity::RaritySource,
) -> Result<core::CharacterMasterData, Error> {
    Ok(core::CharacterMasterData {
        id: value.id,
        title: String::new(),
        name: String::new(),
        rarity: rarity.rarity(value.id)?,
        attribute: enumeration(value.attribute)?,
        hp: value.hp,
        attack: u16::try_from(value.attack).map_err(|_| Error::InvalidValue("attack"))?,
        defense: u16::try_from(value.defense).map_err(|_| Error::InvalidValue("defense"))?,
        speed: enumeration(value.speed)?,
        bp: u16::try_from(value.bp).map_err(|_| Error::InvalidValue("BP"))?,
        skill_ids: value.skill_ids,
        ability_ids: value.ability_ids,
        tactics_ids: value.tactics_ids,
    })
}
pub fn skill(value: wire::SkillMasterData) -> Result<CompositeSkill, Error> {
    let main = component(
        value.id,
        required(value.main_effect)?,
        value.activation_rate,
    )?;
    if main.effect.max_activation_count != value.max_activation_count {
        return Err(Error::InvalidValue("skill activation count"));
    }
    let sub = value
        .sub_effects
        .into_iter()
        .map(|v| component(value.id, v, value.activation_rate))
        .collect::<Result<_, _>>()?;
    let skill = CompositeSkill {
        main_effect: main,
        sub_effects: sub,
        probability_level: value.probability_level,
    };
    skill.validate()?;
    Ok(skill)
}
fn component(
    id: u32,
    value: wire::SkillEffectComponentData,
    activation_rate: f32,
) -> Result<SkillEffectComponent, Error> {
    use core::SkillMasterDataEffectData as C;
    use wire::skill_effect_component_data::EffectData as W;
    let effect_id: core::SkillEffectID = enumeration(required(value.effect_id)?)?;
    // Special codes must have an explicit executor; never reinterpret one as a basic damage effect.
    if value.code.contains("_CHEST_BONUS")
        || value.code.contains("_RAMP")
        || value.code.contains("_ADV_")
    {
        return Err(Error::InvalidValue("unsupported skill modifier"));
    }
    let condition = value.target_condition.unwrap_or_default();
    let mut range = None;
    let data = match value.effect_data {
        Some(W::RandomAttack(v)) => {
            range = Some((v.min_hit_count, v.max_hit_count));
            Some(C::RandomAttack(core::SkillRandomAttackData {
                hit_count: v.min_hit_count,
            }))
        }
        Some(W::StatusAbnormality(v)) => {
            Some(C::StatusAbnormality(core::SkillStatusAbnormalityData {
                status_abnormality_id: enumeration(v.status_abnormality_id)?,
                application_rate: v.application_rate,
            }))
        }
        Some(W::StatCorrection(v)) => Some(C::StatCorrection(core::SkillStatCorrectionData {
            stat_target: enumeration(v.stat_target)?,
        })),
        Some(W::Heal(v)) => Some(C::Heal(core::SkillHealData {
            can_heal_incapacitated: v.can_heal_incapacitated,
            heal_rate: v.heal_rate,
        })),
        None => None,
    };
    let effect = core::SkillMasterData {
        id,
        name: String::new(),
        description: String::new(),
        effect_id,
        activation_rate,
        correction_value: value.correction_value.unwrap_or(0.0),
        target_range: enumeration(required(value.target_range)?)?,
        target_condition: core::SkillTargetConditionData {
            condition_id: enumeration(condition.condition_id)?,
            status_abnormality_id: enumeration(condition.status_abnormality_id)?,
        },
        max_activation_count: if matches!(
            effect_id,
            core::SkillEffectID::Buff | core::SkillEffectID::Debuff
        ) {
            1
        } else {
            u32::MAX
        },
        damage_value_type: enumeration(value.damage_value_type.unwrap_or(0))?,
        target_side: enumeration(required(value.target_side)?)?,
        effect_data: data,
    };
    if matches!(
        effect_id,
        core::SkillEffectID::Attack | core::SkillEffectID::Buff | core::SkillEffectID::Debuff
    ) && value.correction_value.is_none()
    {
        return Err(Error::InvalidValue("missing skill correction"));
    }
    Ok(SkillEffectComponent {
        code: value.code,
        level: value.level,
        effect,
        random_hit_range: range,
    })
}
pub fn ability(value: wire::AbilityMasterData) -> Result<core::AbilityMasterData, Error> {
    use core::AbilityMasterDataEffectData as C;
    use wire::ability_master_data::EffectData as W;
    let condition = required(value.activation_condition)?;
    let data = match required(value.effect_data)? {
        W::Correction(v) => C::Correction(core::AbilityCorrectionData {
            correction_value: v.correction_value,
        }),
        W::NoParameter(_) => C::NoParameter(core::AbilityNoParameterData {}),
        W::StatusAbnormality(v) => C::StatusAbnormality(core::AbilityStatusAbnormalityData {
            status: v.status.map(enumeration).transpose()?,
            avoidance_type: v.avoidance_type.map(enumeration).transpose()?,
        }),
        W::StatCorrection(v) => C::StatCorrection(core::AbilityStatCorrectionData {
            attack: v.attack,
            defense: v.defense,
        }),
        W::IncapacitatedAllyCountStatCorrection(v) => C::IncapacitatedAllyCountStatCorrection(
            core::AbilityIncapacitatedAllyCountStatCorrectionData {
                entries: v
                    .entries
                    .into_iter()
                    .map(|v| core::AbilityIncapacitatedAllyCountStatCorrectionEntry {
                        incapacitated_ally_count: v.incapacitated_ally_count,
                        attack: v.attack,
                        defense: v.defense,
                    })
                    .collect(),
            },
        ),
    };
    Ok(core::AbilityMasterData {
        id: value.id,
        name: value.name,
        description: value.description,
        allowed_attributes: value
            .allowed_attributes
            .into_iter()
            .map(enumeration)
            .collect::<Result<_, _>>()?,
        activation_rate: value.activation_rate,
        max_activation_count: value.max_activation_count,
        activation_condition: core::AbilityActivationConditionData {
            condition_id: enumeration(condition.condition_id)?,
            condition_value: condition.condition_value,
            turn_timing: enumeration(condition.turn_timing)?,
        },
        effect_id: enumeration(value.effect_id)?,
        target: enumeration(value.target)?,
        effect_data: Some(data),
    })
}
pub fn tactics(value: wire::TacticsMasterData) -> Result<core::TacticsMasterData, Error> {
    Ok(core::TacticsMasterData {
        id: value.id,
        name: value.name,
        description: value.description,
        category: enumeration(value.category)?,
        tp_cost: u8::try_from(value.tp_cost).map_err(|_| Error::InvalidValue("TP"))?,
        stage_effects: value
            .stage_effects
            .into_iter()
            .map(stage)
            .collect::<Result<_, _>>()?,
        end_type: enumeration(value.end_type)?,
        duration_seconds: value.duration_seconds,
        effect_count: value.effect_count,
        count_consume_trigger: enumeration(value.count_consume_trigger)?,
        effects: value
            .effects
            .into_iter()
            .map(tactics_effect)
            .collect::<Result<_, _>>()?,
        use_condition: enumeration(value.use_condition)?,
    })
}
fn stage(value: wire::TacticsStageEffectData) -> Result<core::TacticsStageEffectData, Error> {
    use core::TacticsStageEffectDataIncreaseData as C;
    use wire::tactics_stage_effect_data::IncreaseData as W;
    let data = match required(value.increase_data)? {
        W::IncreaseValue(v) => C::IncreaseValue(v),
        W::IncreaseUintValue(v) => C::IncreaseUintValue(v),
        W::BattleSpecialIncrease(v) => C::BattleSpecialIncrease(parameters(v)),
    };
    Ok(core::TacticsStageEffectData {
        effect_id: enumeration(value.effect_id)?,
        target: enumeration(value.target)?,
        effect_index: value.effect_index,
        increase_data: Some(data),
    })
}
fn tactics_effect(value: wire::TacticsEffectData) -> Result<core::TacticsEffectData, Error> {
    use core::TacticsEffectDataEffectData as C;
    use wire::tactics_effect_data::EffectData as W;
    let data = match required(value.effect_data)? {
        W::CorrectionValue(v) => C::CorrectionValue(v),
        W::UintValue(v) => C::UintValue(v),
        W::HpRecovery(v) => C::HpRecovery(core::TacticsHpRecoveryData {
            recovery_type: enumeration(v.recovery_type)?,
            recovery_rate: v.recovery_rate,
        }),
        W::BattleSpecial(v) => C::BattleSpecial(core::TacticsBattleSpecialData {
            special_type: enumeration(v.special_type)?,
            parameters: parameters(required(v.parameters)?),
            trigger: enumeration(v.trigger)?,
        }),
    };
    Ok(core::TacticsEffectData {
        effect_id: enumeration(value.effect_id)?,
        target: enumeration(value.target)?,
        effect_data: Some(data),
    })
}
fn parameters(v: wire::TacticsBattleSpecialParameters) -> core::TacticsBattleSpecialParameters {
    core::TacticsBattleSpecialParameters {
        attack: v.attack,
        defense: v.defense,
        speed: v.speed,
        skill_activation_rate: v.skill_activation_rate,
        max_tp: v.max_tp,
        battle_score: v.battle_score,
        guild_battle_score: v.guild_battle_score,
        castle_break_score: v.castle_break_score,
        assault_castle_break_rate: v.assault_castle_break_rate,
        assault_castle_break_score: v.assault_castle_break_score,
        hate: v.hate,
        castle_level: v.castle_level,
        bp_recovery: v.bp_recovery,
        tp_recovery: v.tp_recovery,
        attack_count_score: v.attack_count_score,
        castle_break_score_limit: v.castle_break_score_limit,
        hp_recovery_value: v.hp_recovery_value,
        revive_rate: v.revive_rate,
        attack_target_rate: v.attack_target_rate,
    }
}
