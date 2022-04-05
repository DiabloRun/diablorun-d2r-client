import * as d2s from "@dschu012/d2s";
import { Payload } from "./types";

// location_id=1 => equipped
// location_id=0 & alt_position_id=1 => inventory
// location_id=0 & alt_position_id=5 => stash
// location_id=0 & alt_position_id=4 => cube

// quality=1 => grey
// quality=2 | quality=3 => white
// quality=4 => blue
// quali

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

export function getItemsPayload(
  items: d2s.types.IItem[]
): Pick<Payload, "ClearItems" | "RemovedItems" | "AddedItems"> {
  return {
    ClearItems: true,
    RemovedItems: [],
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
  };
}
