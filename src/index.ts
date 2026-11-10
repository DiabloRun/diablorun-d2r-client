import { D2RCapture } from "./capture";
import { getConfigPath, loadConfig } from "./config";
import { AppServer } from "./server";
import { Timer } from "./timer";

async function main() {
  const config = loadConfig();
  const configPath = getConfigPath();
  const timer = new Timer();
  let server: AppServer | null = null;

  const capture = new D2RCapture({
    config,
    onLoadingChanged: (loading) => {
      timer.setLoading(loading);
      server?.broadcast();
      console.log(`[detector] ${loading ? "Loading" : "Playing"}`);
    },
    onStateChanged: () => {
      timer.setCaptureAvailable(capture.isAvailable());
      server?.broadcast();
    },
  });

  server = new AppServer({
    config,
    timer,
    capture,
    configPath,
  });

  await server.start();
  capture.start();

  const baseUrl = `http://${config.host}:${config.port}`;

  console.log("");
  console.log(`[server] Listening on ${baseUrl}`);
  console.log(`[server] Overlay: ${baseUrl}/overlay`);
  console.log(`[server] Control: ${baseUrl}/`);
  console.log(`[server] Config: ${configPath}`);
  console.log("");

  let shuttingDown = false;

  const shutdown = async (signal: string) => {
    if (shuttingDown) {
      return;
    }

    shuttingDown = true;
    console.log(`\n[server] ${signal} received, shutting down`);
    capture.stop();

    if (server) {
      await server.close();
    }

    process.exit(0);
  };

  process.on("SIGINT", () => {
    void shutdown("SIGINT");
  });
  process.on("SIGTERM", () => {
    void shutdown("SIGTERM");
  });

  process.on("uncaughtException", (error) => {
    console.error("[process] Uncaught exception:", error);
  });
  process.on("unhandledRejection", (error) => {
    console.error("[process] Unhandled rejection:", error);
  });
}

main().catch((error) => {
  console.error("[server] Failed to start:", error);
  process.exit(1);
});
