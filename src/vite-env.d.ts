/// <reference types="vite/client" />

declare module "*.png" {
  const src: string;
  export default src;
}

/*
 * Minimal shim for the one Node API the tests use. `styles.css?raw` cannot serve this: vitest
 * stubs CSS imports by default, so the raw import comes back as an empty string and every
 * assertion against it passes vacuously. Declared here rather than pulling in `@types/node`,
 * which would be a heavy dependency for a single `readFileSync` in a single test.
 */
declare module "node:fs" {
  export function readFileSync(path: string, encoding: "utf8"): string;
}
