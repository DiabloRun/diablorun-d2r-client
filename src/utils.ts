import * as fs from "fs";
import * as d2s from "@dschu012/d2s";
import constants from "./constants";
import { Payload } from "./payload";
import { mainWindow } from "./index";

export function getItemQuality(qualityId: number) {
  switch (qualityId) {
    default:
      return "white";
    case 4:
      return "blue";

    // yellow, orange, green, gold, blue, socketed, white
  }
}

export async function getPayload(filePath: string, apiKey: string) {
  const buffer = fs.readFileSync(filePath);
  const data = await d2s.read(buffer, constants);

  mainWindow.webContents.send("message", { log: data });

  return d2sDataToPayload(data, apiKey);
}

function d2sDataToPayload(
  {
    attributes,
    corpse_items,
    header,
    item_bonuses,
    items,
    merc_items,
    skills,
  }: d2s.types.ID2S,
  apiKey: string
): Payload {
  return {
    Event: "DataRead",
    Headers: `API_KEY=${apiKey}`,
    DIApplicationInfo: {
      Version: "21.6.16",
    },
    D2ProcessInfo: {
      Type: "D2R",
      Version: `${header.version}`,
      CommandLineArgs: [],
    },

    Seed: 0,
    SeedIsArg: false,
    IsExpansion: header.status.expansion,
    IsHardcore: header.status.hardcore,
    NewCharacter: false,

    Name: header.name,
    Guid: header.identifier,
    CharClass: constants.classes.findIndex((c) => c.n === header.class),

    Area: undefined,
    Difficulty: undefined,
    PlayersX: undefined,

    IsDead: header.status.died ?? false,
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

    Life: attributes.current_hp,
    LifeMax: attributes.max_hp,
    Mana: attributes.current_mana,
    ManaMax: attributes.max_mana,

    FasterCastRate: undefined,
    FasterHitRecovery: undefined,
    FasterRunWalk: undefined,
    IncreasedAttackSpeed: undefined,
    MagicFind: undefined,

    CompletedQuests: {
      Normal: [],
      Nightmare: [],
      Hell: [],
    },

    InventoryTab: undefined,
    ClearItems: true,
    AddedItems: items
      .filter((item) => item.location_id === 1)
      .map((item) => ({
        GUID: 0, // item.id,
        Class: item.type_id,
        BaseItem: item.type_name,
        ItemName: item.type_name,
        Quality: getItemQuality(item.quality),
        Properties: item.displayed_magic_attributes
          .filter(
            (attribute) => attribute.description && attribute.visible !== false
          )
          .map((attribute) => attribute.description),
        Location: {
          X: item.position_x,
          Y: item.position_y,
          Width: item.inv_width,
          Height: item.inv_height,
          BodyLocation: item.equipped_id,
          Container: 0, // item.location_id,
        },
      })),
    RemovedItems: [],

    Hireling: undefined,

    /*
        Hireling?: {
            Name?: string;
            Class?: number;
            Level?: number;
            Experience?: number;
            Strength?: number;
            Dexterity?: number;
            FireResist?: number;
            ColdResist?: number;
            LightningResist?: number;
            PoisonResist?: number;
            SkillIds?: number[];

            Items?: ItemPayload[];
            AddedItems?: ItemPayload[];
            RemovedItems?: ItemPayload[] | number[];
        };
        */

    KilledMonsters: [],
  };
}
