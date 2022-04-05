import * as fs from "fs";
import * as d2s from "@dschu012/d2s";
import settings from "electron-settings";
import constants from "./constants";
import { Payload } from "./payload";
import { mainWindow } from "./index";

export function getItemQuality(qualityId: number) {
  switch (qualityId) {
    default:
      return "white";
    case 1:
      return "socketed";
    case 4:
      return "blue";
    case 5:
      return "green";
    case 6:
      return "yellow";
    case 7:
      return "gold";
    case 8:
      return "orange";
  }
}

export function getItemName(item: d2s.types.IItem) {
  let name = item.type_name;
  if (item.magic_prefix_name) {
    name = `${item.magic_prefix_name} ${name}`;
  }

  if (item.magic_suffix_name) {
    name = `${name} ${item.magic_suffix_name}`;
  }

  if (item.rare_name) {
    name = `${item.rare_name} ${name}`;
  }

  if (item.rare_name2) {
    name = `${name} ${item.rare_name2}`;
  }

  const personalizedName = item.personalized_name
    ? `${item.personalized_name}'s `
    : "";

  /*
  if (item.set_name) {
    name = `${name} - ${personalizedName}${item.set_name}`;
  }
  */

  if (item.unique_name) {
    name = `${personalizedName}${item.unique_name}`;
  }

  if (item.runeword_name) {
    const runes = item.socketed_items
      .map((e) => e.type_name.split(" ")[0])
      .join("");

    name = `${personalizedName}${item.runeword_name} [${runes}]`;
  }

  return name;
}

export function getQuestId(quest: string): number | null {
  switch (quest) {
    case "den_of_evil":
      return 81;
    case "sisters_burial_grounds":
      return 82;
    case "sisters_to_the_slaughter":
      return 12;
    case "the_forgotten_tower":
      return 85;
    case "the_search_for_cain":
      return 84;
    case "tools_of_the_trade":
      return 83;

    case "arcane_sanctuary":
      return 89;
    case "radaments_lair":
      return 86;
    case "tainted_sun":
      return 88;
    case "the_horadric_staff":
      return 87;
    case "the_seven_tombs":
      return 28;
    case "the_summoner":
      return 90;

    case "blade_of_the_old_religion":
      return 93;
    case "khalims_will":
      return 92;
    case "lam_esens_tome":
      return 91;
    case "the_blackened_temple":
      return 95;
    case "the_golden_bird":
      return 94;
    case "the_guardian":
      44;

    case "hellforge":
      return 97;
    case "terrors_end":
      return 52;
    case "the_fallen_angel":
      return 96;

    case "betrayal_of_harrogath":
      return 101;
    case "eve_of_destruction":
      return 80;
    case "prison_of_ice":
      return 100;
    case "rescue_on_mount_arreat":
      return 99;
    case "rite_of_passage":
      return 102;
    case "siege_on_harrogath":
      return 98;
  }

  return null;
}

export function getCompletedQuestsInDifficultyPayload(
  quests: d2s.types.IQuests
): number[] {
  const questIds: number[] = [];
  let act: keyof d2s.types.IQuests;

  for (act in quests) {
    for (const questName in quests[act]) {
      const questId = getQuestId(questName);

      if (questId === null) {
        continue;
      }

      const quest = (quests[act] as any)[questName];

      if (quest.is_completed || quest.is_requirement_completed) {
        questIds.push(questId);
      }
    }
  }

  return questIds;
}

export function getCompletedQuestsPayload(
  header: d2s.types.IHeader
): Payload["CompletedQuests"] {
  return {
    Normal: getCompletedQuestsInDifficultyPayload(header.quests_normal),
    Nightmare: getCompletedQuestsInDifficultyPayload(header.quests_nm),
    Hell: getCompletedQuestsInDifficultyPayload(header.quests_hell),
  };
}

export async function getPayload(filePath: string) {
  const buffer = fs.readFileSync(filePath);
  const data = await d2s.read(buffer, constants);

  mainWindow.webContents.send("message", { log: data });

  return await d2sDataToPayload(data);
}

export async function getPayloadHeader() {
  const { apiKey, raceId } = await settings.get();

  return {
    Headers: `API_KEY=${apiKey}`,
    DIApplicationInfo: {
      Version: "21.6.16",
    },
    D2ProcessInfo: {
      Type: "D2R",
      Version: `d2s`,
      CommandLineArgs: raceId ? [`-race${raceId}`] : [],
    },
  };
}

async function d2sDataToPayload({
  attributes,
  corpse_items,
  header,
  item_bonuses,
  items,
  merc_items,
  skills,
}: d2s.types.ID2S): Promise<Payload> {
  const payloadHeader = await getPayloadHeader();

  return {
    ...payloadHeader,
    Event: "DataRead",
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

    Life: attributes.current_hp ?? 0,
    LifeMax: attributes.max_hp,
    Mana: attributes.current_mana ?? 0,
    ManaMax: attributes.max_mana,

    FasterCastRate: undefined,
    FasterHitRecovery: undefined,
    FasterRunWalk: undefined,
    IncreasedAttackSpeed: undefined,
    MagicFind: undefined,

    CompletedQuests: getCompletedQuestsPayload(header),

    InventoryTab: undefined,
    ClearItems: true,
    AddedItems: items
      .filter((item) => item.location_id === 1)
      .map((item) => ({
        GUID: 0, // item.id,
        Class: item.type_id,
        BaseItem: item.type_name,
        ItemName: getItemName(item),
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
