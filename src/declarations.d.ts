declare module "livesplit-client" {
  export default class LiveSplitClient {
    constructor(url: string);
    connected: boolean;
    connect(): Promise<void>;
    disconnect(): boolean;
    reset(): boolean;
    startOrSplit(): boolean;
    startTimer(): boolean;
    pauseGameTime(): boolean;
    unpauseGameTime(): boolean;
  }
}
