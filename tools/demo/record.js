// Records the console's demo scenes and screenshots against a running server with players on
// it (tools/demo/run.sh starts one, with the `populate` bots playing).
//
//   node tools/demo/record.js --web http://127.0.0.1:8288 --user Boss --password ... --out DIR
//        [--scenes overview,moderate,command-line,restart,dark] [--shots]
//
// Each scene is a video of its own (DIR/<scene>.webm); run.sh turns them into GIF and MP4.
// A pointer is drawn on the page, since recorded video shows no mouse, and commands are typed
// at a person's pace.
"use strict";
const { chromium } = require("playwright");
const fs = require("fs");
const path = require("path");

const args = {};
for (let i = 2; i < process.argv.length; i += 1) {
  const a = process.argv[i];
  if (a.startsWith("--")) {
    const next = process.argv[i + 1];
    if (next === undefined || next.startsWith("--")) args[a.slice(2)] = true;
    else { args[a.slice(2)] = next; i += 1; }
  }
}
const WEB = args.web || "http://127.0.0.1:8288";
const OUT = args.out || "demo-out";
const SIZE = { width: 1280, height: 800 };
const scenes = (args.scenes || "overview,moderate,command-line,restart,dark").split(",");
fs.mkdirSync(OUT, { recursive: true });

// a pointer that follows the mouse and dips when it clicks
const POINTER = `
window.addEventListener("DOMContentLoaded", () => {
  const p = document.createElement("div");
  p.id = "__pointer";
  p.style.cssText = "position:fixed;z-index:99999;left:-40px;top:-40px;width:22px;height:22px;" +
    "border-radius:50%;background:rgba(255,196,0,.55);border:2px solid rgba(255,255,255,.9);" +
    "box-shadow:0 0 0 2px rgba(0,0,0,.35);pointer-events:none;transform:translate(-50%,-50%);" +
    "transition:transform .12s ease";
  document.body.appendChild(p);
  document.addEventListener("mousemove", (e) => { p.style.left = e.clientX + "px"; p.style.top = e.clientY + "px"; }, true);
  document.addEventListener("mousedown", () => { p.style.transform = "translate(-50%,-50%) scale(.7)"; }, true);
  document.addEventListener("mouseup", () => { p.style.transform = "translate(-50%,-50%)"; }, true);
});`;

const pause = (ms) => new Promise((r) => setTimeout(r, ms));

async function moveTo(page, locator) {
  const box = await locator.boundingBox();
  if (!box) throw new Error("not visible: " + locator);
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 28 });
  await pause(180);
}
// scrolls `locator` into the part of the page the command drawer does not cover
async function inView(page, locator) {
  await locator.scrollIntoViewIfNeeded();
  const box = await locator.boundingBox();
  const drawerTop = await page.evaluate(() => document.getElementById("drawer").getBoundingClientRect().top);
  if (box && box.y + box.height > drawerTop - 20) {
    await page.mouse.wheel(0, box.y + box.height - drawerTop + 120);
    await pause(400);
  }
}
async function click(page, locator) {
  await inView(page, locator);
  await moveTo(page, locator);
  // the page may have moved under the pointer (a refresh): the click measures again
  await locator.click({ timeout: 10000 });
  await pause(250);
}
async function typeInto(page, locator, text) {
  await click(page, locator);
  await page.keyboard.type(text, { delay: 65 });
  await pause(250);
}
async function drawerSays(page, text, timeout = 20000) {
  await page.waitForFunction((t) => document.getElementById("out").textContent.includes(t), text, { timeout });
}

async function signedInState(browser) {
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  await page.goto(WEB + "/login");
  await page.fill("input[name=username]", args.user || "Boss");
  await page.fill("input[name=password]", args.password || "");
  await page.click("button[type=submit]");
  await page.waitForURL("**/console");
  const state = await ctx.storageState();
  await ctx.close();
  return state;
}

async function scene(browser, state, name, colorScheme, body) {
  const dir = path.join(OUT, "video-" + name);
  fs.rmSync(dir, { recursive: true, force: true });
  const ctx = await browser.newContext({
    storageState: state,
    viewport: SIZE,
    colorScheme,
    recordVideo: { dir, size: SIZE },
  });
  await ctx.addInitScript(POINTER);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => console.error(name + ": " + e));
  await page.goto(WEB + "/console");
  await page.waitForSelector("#players tbody tr", { timeout: 30000 });
  await page.mouse.move(640, 300);
  await pause(1200);
  try {
    await body(page);
  } catch (e) {
    await page.screenshot({ path: path.join(OUT, name + "-failed.png") });
    console.error(name + ": drawer said: " + (await page.textContent("#out")).slice(-600));
    throw e;
  }
  const video = page.video();
  await ctx.close();
  const file = await video.path();
  fs.renameSync(file, path.join(OUT, name + ".webm"));
  fs.rmSync(dir, { recursive: true, force: true });
  console.log("recorded " + name);
}

// a player around the middle of the list, the same one through a scene
async function aPlayer(page, nth) {
  const rows = page.locator("#players tbody tr");
  const n = await rows.count();
  return rows.nth(Math.min(nth, n - 1));
}

// clicks the nth player and waits for the panel
async function openPlayer(page, nth) {
  for (let tries = 0; tries < 3; tries += 1) {
    await click(page, await aPlayer(page, nth));
    if (await page.locator("#detail").isVisible()) return;
  }
  throw new Error("the player panel did not open");
}

const SCENES = {
  // the live player list: rooms change as they walk
  overview: async (page) => {
    await pause(2500);
    const rows = page.locator("#players tbody tr");
    for (const i of [2, 7, 12]) {
      await moveTo(page, rows.nth(i));
      await pause(900);
    }
    await page.mouse.move(1100, 700, { steps: 20 });
    await pause(3500);
  },
  // a player, muted, jailed and let out again from the panel
  moderate: async (page) => {
    await openPlayer(page, 4);
    await pause(900);
    await typeInto(page, page.locator("#d-reason"), "spamming gossip");
    await click(page, page.locator("[data-act=mute]"));
    await drawerSays(page, "Muted");
    await pause(1500);
    await page.locator("#d-reason").fill("");
    await typeInto(page, page.locator("#d-reason"), "fighting in town");
    await click(page, page.locator("[data-act=jail]"));
    await drawerSays(page, "Jailed");
    await pause(2500);
    await click(page, page.locator("[data-act=release]"));
    await drawerSays(page, "Released");
    await click(page, page.locator("[data-act=unmute]"));
    await drawerSays(page, "Lifted");
    await pause(2000);
  },
  // the command line: the same commands as `silvermere sysop`
  "command-line": async (page) => {
    const name = (await (await aPlayer(page, 9)).locator("td").first().textContent()).split(" ")[0];
    const cmd = page.locator("#cmd");
    for (const line of [`tell ${name} "Welcome to the realm!"`, "moderation", "help jail"]) {
      await typeInto(page, cmd, line);
      await page.keyboard.press("Enter");
      await pause(2200);
    }
    await pause(1500);
  },
  // a restart: the page loses the server, then picks it up again, still signed in
  restart: async (page) => {
    const answers = ["10s", "back in a moment"];
    page.on("dialog", (d) => d.accept(answers.shift() || ""));
    await click(page, page.locator("#b-restart"));
    await drawerSays(page, "will restart");
    await page.waitForFunction(() => !document.getElementById("conn").hidden, null, { timeout: 60000 });
    await drawerSays(page, "The server is back.", 120000);
    await pause(3000);
  },
  // the same in dark mode
  dark: async (page) => {
    await openPlayer(page, 6);
    await pause(3000);
    await page.mouse.move(900, 600, { steps: 20 });
    await pause(2000);
  },
};

(async () => {
  const browser = await chromium.launch();
  const state = await signedInState(browser);
  for (const s of scenes) {
    if (!SCENES[s]) throw new Error("no scene " + s);
    await scene(browser, state, s, s === "dark" ? "dark" : "light", SCENES[s]);
  }
  if (args.shots) {
    for (const [file, opts] of [
      ["console-light.png", { viewport: SIZE, colorScheme: "light", deviceScaleFactor: 2 }],
      ["console-dark.png", { viewport: SIZE, colorScheme: "dark", deviceScaleFactor: 2 }],
      ["console-phone.png", { viewport: { width: 390, height: 844 }, colorScheme: "dark", deviceScaleFactor: 3, isMobile: true }],
    ]) {
      const ctx = await browser.newContext({ storageState: state, ...opts });
      const page = await ctx.newPage();
      await page.goto(WEB + "/console");
      await page.waitForSelector("#players tbody tr", { timeout: 30000 });
      if (file !== "console-phone.png") await page.locator("#players tbody tr").nth(1).click();
      await page.evaluate(() => window.scrollTo(0, 0));
      await pause(1500);
      await page.screenshot({ path: path.join(OUT, file) });
      await ctx.close();
      console.log("shot " + file);
    }
  }
  await browser.close();
})().catch((e) => { console.error(e); process.exit(1); });
