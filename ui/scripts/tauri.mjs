// Runs the Tauri CLI against crates/app (it does not search outside ui/ on its own).
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { run } from "@tauri-apps/cli";

process.env.TAURI_APP_PATH ??= fileURLToPath(new URL("../../crates/app", import.meta.url));

// The app starts arrowhead-overlay from next to its own binary; tauri only builds the app crate.
const cmd = process.argv[2];
if (cmd === "dev" || cmd === "build") {
  const args = ["build", "-p", "arrowhead-overlay", ...(cmd === "build" ? ["--release"] : [])];
  const r = spawnSync("cargo", args, { stdio: "inherit", cwd: fileURLToPath(new URL("../..", import.meta.url)) });
  if (r.status !== 0) process.exit(r.status ?? 1);
}
await run(process.argv.slice(2), "tauri");
