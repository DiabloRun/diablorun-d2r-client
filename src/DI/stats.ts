import * as d2s from "../lib/d2s";
import { Payload } from "./types";

export function getStatsPayload(
  attributes: d2s.types.IAttributes
): Pick<
  Payload,
  | "Deaths"
  | "Level"
  | "Experience"
  | "Strength"
  | "Dexterity"
  | "Vitality"
  | "Energy"
  | "FireResist"
  | "ColdResist"
  | "LightningResist"
  | "PoisonResist"
  | "Gold"
  | "GoldStash"
  | "Life"
  | "LifeMax"
  | "Mana"
  | "ManaMax"
  | "FasterCastRate"
  | "FasterHitRecovery"
  | "FasterRunWalk"
  | "IncreasedAttackSpeed"
  | "MagicFind"
> {
  return {
    Deaths: undefined,

    Level: attributes.level,
    Experience: attributes.experience ?? 0,
    Strength: attributes.strength,
    Dexterity: attributes.dexterity,
    Vitality: attributes.vitality,
    Energy: attributes.energy,

    FireResist: undefined,
    ColdResist: undefined,
    LightningResist: undefined,
    PoisonResist: undefined,

    Gold: attributes.gold ?? 0,
    GoldStash: attributes.stashed_gold ?? 0,

    Life: attributes.current_hp ?? 0,
    LifeMax: attributes.max_hp,
    Mana: attributes.current_mana ?? 0,
    ManaMax: attributes.max_mana,

    FasterCastRate: undefined,
    FasterHitRecovery: undefined,
    FasterRunWalk: undefined,
    IncreasedAttackSpeed: undefined,
    MagicFind: undefined,
  };
}
