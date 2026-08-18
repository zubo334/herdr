"use strict";

const { execSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const POLL_INTERVAL_MS = 60_000;

const PLATFORMS = [
  { kind: "claude", name: "usage-poller-claude", label: "_monitor-claude", command: "/usage", closeKey: "Escape" },
  { kind: "codex", name: "usage-poller-codex", label: "_monitor-codex", commands: ["/status", "/usage"], closeKeys: [null, "Escape"] },
];

function shellQuote(value) {
  const text = String(value);
  if (process.platform === "win32") {
    return `"${text.replace(/%/g, "%%").replace(/"/g, '""')}"`;
  }
  return `'${text.replace(/'/g, `'\\''`)}'`;
}

function runHerdr(args, timeoutMs = 30_000) {
  const herdr = process.env.HERDR_BIN_PATH;
  if (!herdr) throw new Error("HERDR_BIN_PATH required");
  return execSync([herdr, ...args].map(shellQuote).join(" "), {
    encoding: "utf8",
    env: process.env,
    maxBuffer: 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
    timeout: timeoutMs,
  }).trim();
}

function sleep(ms) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

function findPollerPane(agentName) {
  try {
    const result = JSON.parse(runHerdr(["agent", "list"]));
    return (result?.result?.agents || []).find((a) => a.name === agentName) || null;
  } catch (_) {
    return null;
  }
}

function findMonitorWorkspace() {
  try {
    const result = JSON.parse(runHerdr(["workspace", "list"]));
    return (result?.result?.workspaces || []).find(
      (w) => w.label === "_monitor",
    ) || null;
  } catch (_) {
    return null;
  }
}

function ensureMonitorWorkspace() {
  const existing = findMonitorWorkspace();
  if (existing) return existing.workspace_id;

  try {
    const result = JSON.parse(
      runHerdr(["workspace", "create", "--label", "_monitor", "--no-focus"]),
    );
    return result?.result?.workspace?.workspace_id || null;
  } catch (_) {
    return null;
  }
}

function startPollerForPlatform(platform, wsId) {
  try {
    // create a new tab in the _monitor workspace so each agent gets full viewport
    const tabResult = JSON.parse(
      runHerdr(["tab", "create", "--workspace", wsId, "--no-focus"]),
    );
    const paneId = tabResult?.result?.pane?.pane_id || tabResult?.result?.root_pane?.pane_id;
    if (!paneId) return null;

    runHerdr(["pane", "rename", paneId, platform.label]);
    runHerdr(["agent", "start", platform.name, "--kind", platform.kind, "--pane", paneId, "--timeout", "60000"], 90_000);
    return paneId;
  } catch (e) {
    process.stderr.write(`usage-poller: start ${platform.kind} failed: ${e.stderr?.slice(0, 200) || e.message}\n`);
    return null;
  }
}

function waitForIdle(paneId, timeoutMs = 30_000) {
  try {
    runHerdr(["agent", "wait", paneId, "--until", "idle", "--timeout", String(timeoutMs)], timeoutMs + 5_000);
    return true;
  } catch (_) {
    return false;
  }
}

function parseClaudeUsage(text) {
  const result = { session_percent: null, weekly_percent: null, weekly_fable_percent: null, session_resets: null, weekly_resets: null };
  const s = text.match(/Current session[^\n]*\n[^\n]*?(\d+)%\s*used/i);
  if (s) result.session_percent = Number(s[1]);
  const sessionBlock = text.match(/Current session([\s\S]*?)Current week/i);
  if (sessionBlock) {
    const sr = sessionBlock[1].match(/Resets\s+([^\n]+)/i);
    if (sr) result.session_resets = sr[1].trim();
  }
  const w = text.match(/Current week \(all models\)[^\n]*\n[^\n]*?(\d+)%\s*used/i);
  if (w) result.weekly_percent = Number(w[1]);
  const weeklyBlock = text.match(/Current week \(all models\)([\s\S]*?)(?:Current week \(|What's contributing)/i);
  if (weeklyBlock) {
    const wr = weeklyBlock[1].match(/Resets\s+([^\n]+)/i);
    if (wr) result.weekly_resets = wr[1].trim();
  }
  const f = text.match(/Current week \(Fable\)[^\n]*\n[^\n]*?(\d+)%\s*used/i);
  if (f) result.weekly_fable_percent = Number(f[1]);
  return result;
}

function parseCodexUsage(text) {
  const result = { session_percent: null, weekly_percent: null, reset_credits: null };
  // /status shows "Weekly limit: [███░] 94% left (resets ...)"
  const weeklyLeft = text.match(/Weekly limit:[^\n]*?(\d+)%\s*left/i);
  if (weeklyLeft) result.weekly_percent = 100 - Number(weeklyLeft[1]);
  // bottom bar shows "weekly 94% left"
  if (result.weekly_percent == null) {
    const bar = text.match(/weekly\s+(\d+)%\s*left/i);
    if (bar) result.weekly_percent = 100 - Number(bar[1]);
  }
  // bottom bar shows "Context 100% left"
  const ctx = text.match(/Context\s+(\d+)%\s*left/i);
  if (ctx) result.session_percent = 100 - Number(ctx[1]);
  // /usage menu shows "You have N usage limit reset(s) available."
  const resets = text.match(/You have (\d+) usage limit reset/i);
  if (resets) result.reset_credits = Number(resets[1]);
  // "No usage limit resets available" = 0
  if (result.reset_credits == null && /No usage limit resets available/i.test(text)) {
    result.reset_credits = 0;
  }
  return result;
}

function readPane(paneId) {
  try {
    return runHerdr(["agent", "read", paneId, "--source", "detection", "--format", "text"]);
  } catch (_) {
    return null;
  }
}

function sendCommand(paneId, command, closeKey) {
  try {
    runHerdr(["agent", "prompt", paneId, command, "--wait", "--until", "idle"]);
  } catch (_) {}
  sleep(2000);
  const output = readPane(paneId);
  if (closeKey) {
    try { runHerdr(["pane", "send-keys", paneId, closeKey]); } catch (_) {}
    sleep(1000);
  }
  return output;
}

function pollPlatform(paneId, platform) {
  if (!waitForIdle(paneId, 10_000)) return null;

  const commands = platform.commands || [platform.command];
  const closeKeys = platform.closeKeys || [platform.closeKey];

  let combined = "";
  for (let i = 0; i < commands.length; i++) {
    if (i > 0 && !waitForIdle(paneId, 10_000)) break;
    const output = sendCommand(paneId, commands[i], closeKeys[i] || null);
    if (output) combined += "\n" + output;
  }

  if (!combined) return null;
  const parser = platform.kind === "codex" ? parseCodexUsage : parseClaudeUsage;
  return parser(combined);
}

function saveAccountUsage(stateDir, platformName, data) {
  const usagePath = path.join(stateDir, "account-usage.json");
  let usage;
  try { usage = JSON.parse(fs.readFileSync(usagePath, "utf8")); } catch (_) { usage = { version: 1, platforms: {} }; }
  usage.platforms[platformName] = { ...data, updated_at: new Date().toISOString() };
  usage.updated_at = new Date().toISOString();
  const tmp = `${usagePath}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(usage, null, 2) + "\n");
  fs.renameSync(tmp, usagePath);
}

function main() {
  const stateDir = process.env.HERDR_PLUGIN_STATE_DIR;
  if (!stateDir) { process.stderr.write("usage-poller: HERDR_PLUGIN_STATE_DIR required\n"); process.exit(1); }
  fs.mkdirSync(stateDir, { recursive: true });

  const wsId = ensureMonitorWorkspace();
  if (!wsId) { process.stderr.write("usage-poller: failed to create _monitor workspace\n"); process.exit(1); }

  function ensurePaneForPlatform(platform) {
    const existing = findPollerPane(platform.name);
    if (existing) return existing.pane_id;
    const paneId = startPollerForPlatform(platform, wsId);
    if (paneId && waitForIdle(paneId, 60_000)) return paneId;
    return null;
  }

  function poll() {
    for (const platform of PLATFORMS) {
      try {
        const paneId = ensurePaneForPlatform(platform);
        if (!paneId) continue;
        const data = pollPlatform(paneId, platform);
        if (data && (data.session_percent != null || data.weekly_percent != null)) {
          saveAccountUsage(stateDir, platform.kind, data);
        }
      } catch (e) {
        process.stderr.write(`usage-poller: ${platform.kind} poll error: ${e.message}\n`);
      }
    }
  }

  poll();
  setInterval(poll, POLL_INTERVAL_MS);
}

if (require.main === module) {
  try { main(); } catch (e) {
    process.stderr.write(`usage-poller: ${e.message}\n`);
    process.exitCode = 1;
  }
}
