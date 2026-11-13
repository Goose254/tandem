// Screenshots of every UI state, from the demo build and the simulated backend.
//
//   npm run build:demo
//   node scripts/screenshots.mjs            # everything
//   node scripts/screenshots.mjs later      # only the states whose name contains "later"
//   node scripts/screenshots.mjs --capture  # re-record docs/images/auto-sync.gif and .mp4
//
// Each state is opened in a headless Microsoft Edge (already on every Windows machine: no
// browser to download, no new dependency) at the two window sizes the app is checked at, in
// both languages and both themes, and saved under screens/ (git-ignored). A contact sheet per
// size, language and theme shows them all at a glance.
//
// The states come from the page switches of the simulated backend (src/lib/mock.ts):
// `?sync=`, `?outputs=`, `?view=`, `?update=`, `?autostart=`.
//
// `--capture` records the README's auto-sync clip the same way: the page starts a simulated
// run by itself (`?autorun=1`), one frame is taken per 40 ms of the page's own clock, and
// ffmpeg (which must be on the PATH) assembles the frames. Nothing is played or measured.

import { spawn, spawnSync } from "node:child_process";
import { createReadStream, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { extname, join, resolve } from "node:path";

const ROOT = resolve("dist-demo");
const OUT = resolve("screens");
const EDGE = [
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
  "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
].find(existsSync);

const SIZES = [
  [1040, 720],
  [820, 560],
];
const LANGS = ["en", "fr"];
const THEMES = ["light", "dark"];

const SYNC = [
  "preparing", "first", "second", "analysing", "pass2", "refFirst", "refSecond",
  "later", "earlier", "same", "referenceMoved", "limited", "automatic",
  "noMic", "micMissing", "micFailed", "notRunning", "referenceNotRunning", "interrupted",
  "otherAudio", "sourceNotHeard", "targetNotHeard", "tooShort", "busy",
  "blockedNoMic", "blockedMicMissing", "blockedNotPlaying", "blockedBusy",
];
const OUTPUTS = [
  "sourceWaiting", "sourceInUse", "sourceInvalidated", "sourceStalled", "sourceOther",
  "outputInUse", "outputInvalidated", "outputStalled", "outputOther", "outputWaiting",
  "isSource", "dropouts", "empty", "autoResynced", "autoFailed",
];

/** Every state: a file name and the page switches that show it. */
const STATES = [
  { name: "outputs", query: "view=outputs" },
  { name: "sync", query: "view=sync" },
  { name: "settings", query: "view=settings" },
  { name: "settings-autostart-refused", query: "view=settings&autostart=fail" },
  { name: "update-available", query: "update=9.9.9", wait: 8000 },
  { name: "tray", query: "window=tray" },
  ...SYNC.map((s) => ({ name: `sync-${s}`, query: `sync=${s}` })),
  ...OUTPUTS.map((s) => ({ name: `outputs-${s}`, query: `outputs=${s}` })),
];

const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".png": "image/png" };

function serve(root) {
  return new Promise((ready) => {
    const server = createServer((req, res) => {
      const path = decodeURIComponent(new URL(req.url, "http://x").pathname);
      const file = join(root, path === "/" ? "index.html" : path);
      if (!file.startsWith(root) || !existsSync(file)) {
        res.writeHead(404).end();
        return;
      }
      res.writeHead(200, { "content-type": TYPES[extname(file)] ?? "application/octet-stream" });
      createReadStream(file).pipe(res);
    });
    server.listen(0, "127.0.0.1", () => ready(server));
  });
}

/** One headless Edge run that saves a screenshot. Virtual time lets timers and animations finish. */
function shoot(profile, url, file, [width, height], wait) {
  return new Promise((done, fail) => {
    const edge = spawn(
      EDGE,
      [
        "--headless=new",
        `--user-data-dir=${profile}`,
        "--no-first-run",
        "--disable-gpu",
        "--hide-scrollbars",
        "--force-device-scale-factor=1",
        `--window-size=${width},${height}`,
        `--virtual-time-budget=${wait}`,
        `--screenshot=${file}`,
        url,
      ],
      { stdio: "ignore" },
    );
    edge.on("error", fail);
    edge.on("exit", () => (existsSync(file) ? done() : fail(new Error(`no screenshot for ${url}`))));
  });
}

async function pool(jobs, workers) {
  let next = 0;
  let finished = 0;
  await Promise.all(
    Array.from({ length: workers }, async (_, worker) => {
      while (next < jobs.length) {
        const job = jobs[next++];
        await job(worker);
        finished += 1;
        if (finished % 25 === 0 || finished === jobs.length) console.log(`  ${finished}/${jobs.length}`);
      }
    }),
  );
}

/** A page of every screenshot of one combination, itself screenshot into one image. */
function sheetHtml(dir, states, [width, height]) {
  const scale = 0.5;
  const cells = states
    .map(
      (s) =>
        `<figure><img src="file:///${join(dir, `${s.name}.png`).replaceAll("\\", "/")}" width="${width * scale}" height="${height * scale}"><figcaption>${s.name}</figcaption></figure>`,
    )
    .join("");
  return `<!doctype html><meta charset="utf-8"><style>
    body{margin:0;padding:16px;background:#777;font:12px system-ui;display:grid;grid-template-columns:repeat(4,max-content);gap:14px}
    figure{margin:0}img{display:block}figcaption{color:#fff;padding:3px 0}
  </style>${cells}`;
}

if (!EDGE) {
  console.error("Microsoft Edge was not found.");
  process.exit(1);
}
if (!existsSync(join(ROOT, "index.html"))) {
  console.error("dist-demo/ is missing: run `npm run build:demo` first.");
  process.exit(1);
}

const server = await serve(ROOT);
const base = `http://127.0.0.1:${server.address().port}/`;
const profiles = mkdtempSync(join(tmpdir(), "tandem-screens-"));
const WORKERS = 6;

if (process.argv[2] === "--capture") {
  // 11.6 s at 25 frames per second: the run starts at 0.7 s and ends at 8 s.
  const FRAMES = 290;
  const dir = join(OUT, "capture");
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  const url = `${base}?view=sync&autorun=1&theme=light`;
  console.log(`${FRAMES} frames into ${dir}`);
  await pool(
    Array.from({ length: FRAMES }, (_, i) => (worker) =>
      shoot(join(profiles, `p${worker}`), url, join(dir, `f${String(i).padStart(4, "0")}.png`), SIZES[0], 200 + i * 40),
    ),
    WORKERS,
  );
  const frames = ["-y", "-loglevel", "error", "-framerate", "25", "-i", join(dir, "f%04d.png")];
  const images = resolve("docs", "images");
  const made = [
    spawnSync("ffmpeg", [...frames, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "27", "-movflags", "+faststart", join(images, "auto-sync.mp4")], { stdio: "inherit" }),
    spawnSync("ffmpeg", [...frames, "-vf", "fps=10,scale=800:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=96[p];[b][p]paletteuse=dither=bayer:bayer_scale=4", join(images, "auto-sync.gif")], { stdio: "inherit" }),
  ];
  server.close();
  rmSync(profiles, { recursive: true, force: true });
  if (made.some((m) => m.status !== 0)) {
    console.error("ffmpeg failed (is it on the PATH?)");
    process.exit(1);
  }
  console.log("docs/images/auto-sync.mp4 and auto-sync.gif written");
  process.exit(0);
}

const only = process.argv[2];
const states = only ? STATES.filter((s) => s.name.includes(only)) : STATES;

const jobs = [];
const combos = [];
for (const size of SIZES) {
  for (const lang of LANGS) {
    for (const theme of THEMES) {
      const dir = join(OUT, `${size[0]}x${size[1]}`, `${lang}-${theme}`);
      mkdirSync(dir, { recursive: true });
      combos.push({ size, lang, theme, dir });
      for (const state of states) {
        const url = `${base}?${state.query}&lang=${lang}&theme=${theme}`;
        // The tray popup is its own small window.
        const shot = state.name === "tray" ? [320, 360] : size;
        jobs.push((worker) =>
          shoot(join(profiles, `p${worker}`), url, join(dir, `${state.name}.png`), shot, state.wait ?? 3000),
        );
      }
    }
  }
}

console.log(`${jobs.length} screenshots into ${OUT}`);
await pool(jobs, WORKERS);

console.log("contact sheets");
const sheets = combos.map(({ size, lang, theme, dir }) => async (worker) => {
  const page = join(dir, "sheet.html");
  writeFileSync(page, sheetHtml(dir, states, size));
  const rows = Math.ceil(states.length / 4);
  const sheet = [4 * (size[0] / 2 + 14) + 32, rows * (size[1] / 2 + 34) + 32];
  await shoot(
    join(profiles, `p${worker}`),
    `file:///${page.replaceAll("\\", "/")}`,
    join(OUT, `sheet-${size[0]}x${size[1]}-${lang}-${theme}.png`),
    sheet,
    1500,
  );
});
await pool(sheets, WORKERS);

server.close();
rmSync(profiles, { recursive: true, force: true });
console.log("done");
