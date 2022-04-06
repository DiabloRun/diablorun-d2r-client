export interface Settings {
  apiKey: string;
  savesDir: string;
  raceId: string;
  liveSplitServer: string;
  liveSplitEnabled: boolean;
}

export interface IpcMessage {
  settings?: Settings;
  log?: any;
  captureSourceId?: string;
  forceUnpauseGameTime?: number;
  liveSplitStatus?: string;
}
