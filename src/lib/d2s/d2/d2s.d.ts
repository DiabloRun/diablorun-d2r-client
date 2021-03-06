import * as types from "./types";
import { BitReader } from "../binary/bitreader";
import { BitWriter } from "../binary/bitwriter";
declare function reader(buffer: Uint8Array): BitReader;
declare function read(buffer: Uint8Array, constants: types.IConstantData, userConfig?: types.IConfig): Promise<types.ID2S>;
declare function readItem(buffer: Uint8Array, version: number, constants: types.IConstantData, userConfig?: types.IConfig): Promise<types.IItem>;
declare function writer(buffer: Uint8Array): BitWriter;
declare function write(data: types.ID2S, constants: types.IConstantData, userConfig?: types.IConfig): Promise<Uint8Array>;
declare function writeItem(item: types.IItem, version: number, constants: types.IConstantData, userConfig?: types.IConfig): Promise<Uint8Array>;
export { reader, writer, read, write, readItem, writeItem };
