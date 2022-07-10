import * as d2s from "../lib/d2s";
import { Payload } from "./types";

function getQuestId(quest: string): number | null {
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

function getCompletedQuestsInDifficultyPayload(
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

export function getQuestsPayload(
  header: d2s.types.IHeader
): Pick<Payload, "CompletedQuests"> {
  return {
    CompletedQuests: {
      Normal: getCompletedQuestsInDifficultyPayload(header.quests_normal),
      Nightmare: getCompletedQuestsInDifficultyPayload(header.quests_nm),
      Hell: getCompletedQuestsInDifficultyPayload(header.quests_hell),
    },
  };
}
