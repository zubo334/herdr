"use strict";

const { execSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const os = require("node:os");

const CLAUDE_PROJECTS_DIR = path.join(os.homedir(), ".claude", "projects");

function shellQuote(value) {
  const text = String(value);
  if (process.platform === "win32") {
    return `"${text.replace(/%/g, "%%").replace(/"/g, '""')}"`;
  }
  return `'${text.replace(/'/g, `'\\''`)}'`;
}

function runHerdr(herdr, args) {
  const command = [herdr, ...args].map(shellQuote).join(" ");
  return execSync(command, {
    encoding: "utf8",
    env: process.env,
    maxBuffer: 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  }).trim();
}

function cwdToProjectDir(cwd) {
  const normalized = cwd.replace(/[:\\]/g, (ch) => (ch === ":" ? "" : "-"));
  return normalized.replace(/\//g, "-");
}

const CODEX_SESSIONS_DIR = path.join(os.homedir(), ".codex", "sessions");

function findCodexSessionLog(sessionId) {
  try {
    const walk = (dir) => {
      for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory()) {
          const found = walk(full);
          if (found) return found;
        } else if (entry.name.includes(sessionId) && entry.name.endsWith(".jsonl")) {
          return full;
        }
      }
      return null;
    };
    return walk(CODEX_SESSIONS_DIR);
  } catch (_) {
    return null;
  }
}

function findSessionLog(cwd, sessionId) {
  const projectDir = path.join(CLAUDE_PROJECTS_DIR, cwdToProjectDir(cwd));
  const logFile = path.join(projectDir, `${sessionId}.jsonl`);
  if (fs.existsSync(logFile)) return logFile;

  // try subdirectories if direct mapping misses
  try {
    for (const entry of fs.readdirSync(CLAUDE_PROJECTS_DIR)) {
      const candidate = path.join(CLAUDE_PROJECTS_DIR, entry, `${sessionId}.jsonl`);
      if (fs.existsSync(candidate)) return candidate;
    }
  } catch (_) {}
  return null;
}

function parseSessionContext(logPath) {
  const content = fs.readFileSync(logPath, "utf8");
  const lines = content.split("\n").filter(Boolean);

  let lastInputTokens = null;
  let totalOutputTokens = 0;
  let totalCost = 0;
  let model = null;

  for (const line of lines) {
    let entry;
    try { entry = JSON.parse(line); } catch (_) { continue; }

    // Claude format: message.usage
    const usage = entry.message?.usage;
    if (usage) {
      if (usage.input_tokens != null) {
        lastInputTokens = usage.input_tokens + (usage.cache_read_input_tokens || 0) + (usage.cache_creation_input_tokens || 0);
      }
      if (usage.output_tokens != null) totalOutputTokens += usage.output_tokens;
      if (entry.message?.model) model = entry.message.model;
    }

    // Codex format: event_msg with token_count payload
    const payload = entry.payload || entry;
    if (payload.type === "token_count" && payload.total_token_usage) {
      const tu = payload.total_token_usage;
      lastInputTokens = (tu.input_tokens || 0) + (tu.cached_input_tokens || 0);
      totalOutputTokens = tu.output_tokens || 0;
    }

    if (entry.costUSD != null) totalCost += entry.costUSD;
  }

  return { lastInputTokens, totalOutputTokens, totalCost, model };
}

function formatContextToken(ctx) {
  if (ctx.lastInputTokens == null) return null;
  const total = ctx.lastInputTokens;
  if (total >= 1_000_000) return `${(total / 1_000_000).toFixed(1)}M tokens`;
  if (total >= 1_000) return `${(total / 1_000).toFixed(0)}K tokens`;
  return `${total} tokens`;
}

function main() {
  const envelope = JSON.parse(process.env.HERDR_PLUGIN_EVENT_JSON || "null");
  const event = envelope?.data || envelope;

  const paneId = event?.pane_id;
  if (!paneId) return;

  const herdr = process.env.HERDR_BIN_PATH;
  if (!herdr) return;

  let info;
  try {
    const response = JSON.parse(runHerdr(herdr, ["agent", "get", paneId]));
    info = response?.result?.agent;
    if (!info) return;
  } catch (_) { return; }

  const sessionId = info.agent_session?.value;
  const cwd = info.cwd;
  if (!sessionId || !cwd) return;

  const agentType = String(info.agent || "").toLowerCase();

  let logPath;
  if (agentType.includes("claude")) {
    logPath = findSessionLog(cwd, sessionId);
  } else if (agentType.includes("codex")) {
    logPath = findCodexSessionLog(sessionId);
  } else {
    return;
  }
  if (!logPath) return;

  const ctx = parseSessionContext(logPath);
  const tokenLabel = formatContextToken(ctx);
  if (!tokenLabel) return;

  try {
    runHerdr(herdr, [
      "pane", "report-metadata", paneId,
      "--source", "usage-monitor",
      "--token", `context_usage=${tokenLabel}`,
    ]);
  } catch (_) {}

  // also save to usage.json for the dashboard
  const stateDir = process.env.HERDR_PLUGIN_STATE_DIR;
  if (stateDir) {
    fs.mkdirSync(stateDir, { recursive: true });
    const usagePath = path.join(stateDir, "usage.json");
    let usage;
    try { usage = JSON.parse(fs.readFileSync(usagePath, "utf8")); } catch (_) { usage = { version: 1, agents: {} }; }
    const name = info.terminal_title_stripped || info.name || paneId;
    usage.agents[paneId] = {
      pane_id: paneId,
      name,
      agent: info.agent || "claude",
      agent_type: "claude",
      model: ctx.model,
      context_tokens: ctx.lastInputTokens,
      total_output_tokens: ctx.totalOutputTokens,
      cost: ctx.totalCost > 0 ? ctx.totalCost.toFixed(4) : null,
      updated_at: new Date().toISOString(),
    };
    usage.updated_at = new Date().toISOString();
    const tmp = `${usagePath}.${process.pid}.tmp`;
    fs.writeFileSync(tmp, JSON.stringify(usage, null, 2) + "\n");
    fs.renameSync(tmp, usagePath);
  }
}

if (require.main === module) {
  try { main(); } catch (e) {
    process.stderr.write(`usage-monitor: ${e.message}\n`);
    process.exitCode = 1;
  }
}
