import * as fs from "fs";
import * as d2s from "@dschu012/d2s";
import settings from "electron-settings";
import constants from "./constants";
import { getItemsPayload } from "./items";
import { getQuestsPayload } from "./quests";
import { Payload } from "./types";
import { getStatsPayload } from "./stats";

export async function getPayloadHeader(event: "ProcessFound" | "DataRead") {
  const { apiKey, raceId } = await settings.get();

  return {
    Event: event,
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
  const payloadHeader = await getPayloadHeader("DataRead");
  const statsPayload = getStatsPayload(attributes);
  const itemsPayload = getItemsPayload(items);
  const questsPayload = getQuestsPayload(header);

  return {
    ...payloadHeader,

    Seed: 0,
    SeedIsArg: false,
    NewCharacter: false,

    Name: header.name,
    Guid: header.identifier,
    CharClass: constants.classes.findIndex((c) => c.n === header.class),

    IsExpansion: header.status.expansion,
    IsHardcore: header.status.hardcore,
    IsDead: header.status.died ?? false,

    Area: undefined,
    Difficulty: undefined,
    PlayersX: undefined,

    InventoryTab: undefined,
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

    ...statsPayload,
    ...questsPayload,
    ...itemsPayload,
  };
}

export async function getPayload(filePath: string) {
  const buffer = fs.readFileSync(filePath);
  const data = await d2s.read(buffer, constants);

  return await d2sDataToPayload(data);
}
