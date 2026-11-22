// Runs the Tauri CLI against crates/app (it does not search outside ui/ on its own).
import { fileURLToPath } from "node:url";
import { run } from "@tauri-apps/cli";

process.env.TAURI_APP_PATH ??= fileURLToPath(new URL("../../crates/app", import.meta.url));
await run(process.argv.slice(2), "tauri");
