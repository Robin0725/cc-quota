import { readFileSync } from "node:fs";

const packageVersion = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8")).version;
const tauriVersion = JSON.parse(readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8")).version;
const cargoToml = readFileSync(new URL("../src-tauri/Cargo.toml", import.meta.url), "utf8");
const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1];

if (!cargoVersion || packageVersion !== cargoVersion || packageVersion !== tauriVersion) {
  throw new Error(`Version mismatch: package=${packageVersion}, cargo=${cargoVersion ?? "missing"}, tauri=${tauriVersion}`);
}

const tag = process.env.GITHUB_REF_TYPE === "tag" ? process.env.GITHUB_REF_NAME : process.argv[2];
if (tag && tag !== `v${packageVersion}`) {
  throw new Error(`Tag ${tag} does not match manifest version v${packageVersion}`);
}

console.log(`Version ${packageVersion} is consistent${tag ? ` with tag ${tag}` : ""}.`);
