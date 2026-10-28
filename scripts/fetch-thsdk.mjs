// Fetches thsdk's runtime, which signs in to a Tonghuashun account and reads
// its watchlist (src-tauri/src/market/ths/thsdk.rs), into src-tauri/thsdk/:
// the library for this platform and its license. It comes from thsdk's
// release on PyPI, pinned by hash. Runs after `pnpm install`, since building
// the app (even `cargo check`) needs the library where the Tauri config
// names it. Does nothing once the files are there.

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { gunzipSync } from "node:zlib";

const VERSION = "2.0.2";
const URL =
  "https://files.pythonhosted.org/packages/e4/54/53227fca0b82e93e6292e0703ab412e517c87c632f744e82977738da23f3/thsdk-2.0.2.tar.gz";
const SHA256 = "12b950a81df0ddc80f577dca3aed11660e8f0c5ef59b5ef4a21846ae3915f40a";

/** The library each platform the app ships for loads. */
const LIBRARIES = {
  "darwin-arm64": ["libs/darwin/arm64/hq.dylib", "hq.dylib"],
  "win32-x64": ["libs/windows/amd64/hq.dll", "hq.dll"],
};

const out = join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri", "thsdk");
const stamp = join(out, "VERSION");
const library = LIBRARIES[`${process.platform}-${process.arch}`];

if (!library) {
  console.log(`thsdk: no runtime for ${process.platform}-${process.arch}; skipped`);
  process.exit(0);
}
const [inside, name] = library;
const wanted = new Map([
  [`thsdk-${VERSION}/thsdk/${inside}`, name],
  [`thsdk-${VERSION}/LICENSE`, "LICENSE"],
]);
const current =
  existsSync(stamp) &&
  readFileSync(stamp, "utf8").trim() === SHA256 &&
  [...wanted.values()].every((file) => existsSync(join(out, file)));
if (current) process.exit(0);

console.log(`thsdk: fetching ${VERSION}`);
const response = await fetch(URL);
if (!response.ok) throw new Error(`thsdk: ${URL} answered ${response.status}`);
const archive = Buffer.from(await response.arrayBuffer());
const hash = createHash("sha256").update(archive).digest("hex");
if (hash !== SHA256) throw new Error(`thsdk: expected sha256 ${SHA256}, got ${hash}`);

mkdirSync(out, { recursive: true });
let found = 0;
for (const [path, data] of entries(gunzipSync(archive))) {
  const file = wanted.get(path);
  if (!file) continue;
  writeFileSync(join(out, file), data, { mode: file === "LICENSE" ? 0o644 : 0o755 });
  found += 1;
}
if (found !== wanted.size) throw new Error(`thsdk: ${URL} lacks ${[...wanted.keys()].join(" or ")}`);
writeFileSync(stamp, `${SHA256}\n`);
console.log(`thsdk: ${name} and its license in ${out}`);

/** The files of a tar archive, as `[path, contents]`; PAX and GNU long names included. */
function* entries(tar) {
  let offset = 0;
  let longName = null;
  while (offset + 512 <= tar.length) {
    const header = tar.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) return;
    const field = (start, length) => header.subarray(start, start + length).toString("utf8").replace(/\0.*$/s, "");
    const size = parseInt(field(124, 12).trim() || "0", 8);
    const type = field(156, 1);
    const prefix = field(345, 155);
    const data = tar.subarray(offset + 512, offset + 512 + size);
    offset += 512 + Math.ceil(size / 512) * 512;
    if (type === "x") {
      const path = data.toString("utf8").match(/^\d+ path=(.*)$/m);
      longName = path ? path[1] : null;
      continue;
    }
    if (type === "L") {
      longName = data.toString("utf8").replace(/\0.*$/s, "");
      continue;
    }
    const path = longName ?? (prefix ? `${prefix}/${field(0, 100)}` : field(0, 100));
    longName = null;
    if (type === "0" || type === "") yield [path, data];
  }
}
