"use strict";

const fs = require("node:fs");
const path = require("node:path");

const ANSI = {
  clear: "\x1b[2J\x1b[H",
  cyan: "\x1b[36m",
  dim: "\x1b[2m",
  green: "\x1b[32m",
  hideCursor: "\x1b[?25l",
  red: "\x1b[31m",
  reset: "\x1b[0m",
  showCursor: "\x1b[?25h",
  yellow: "\x1b[33m",
};

function truncate(value, width) {
  const text = String(value ?? "—");
  if (text.length <= width) {
    return text.padEnd(width);
  }
  return `${text.slice(0, Math.max(0, width - 1))}…`;
}

function formatPercent(value) {
  return Number.isFinite(value) ? `${value.toFixed(1)}%` : "—";
}

function readUsage(usagePath) {
  try {
    const usage = JSON.parse(fs.readFileSync(usagePath, "utf8"));
    return { usage, error: null };
  } catch (error) {
    if (error.code === "ENOENT") {
      return { usage: { agents: {}, updated_at: null }, error: null };
    }
    return { usage: { agents: {}, updated_at: null }, error: error.message };
  }
}

function render(usagePath) {
  const { usage, error } = readUsage(usagePath);
  const agents = Object.values(usage.agents || {}).sort((left, right) =>
    String(left.name || left.pane_id).localeCompare(String(right.name || right.pane_id)),
  );

  const terminalWidth = Math.max(55, Math.min(process.stdout.columns || 90, 140));
  const sep = "  ";
  const nameW = terminalWidth >= 90 ? 20 : 14;
  const modelW = terminalWidth >= 90 ? 22 : 14;
  const sessW = 10;
  const weekW = 10;
  const costW = 10;
  const lineWidth = nameW + modelW + sessW + weekW + costW + 8;
  const lines = [];

  lines.push(`${ANSI.cyan}Token Usage Dashboard${ANSI.reset}`);
  lines.push(
    `${ANSI.dim}Refreshes every 5s \xb7 q/Esc closes \xb7 Last update: ${
      usage.updated_at ? new Date(usage.updated_at).toLocaleTimeString() : "waiting for data"
    }${ANSI.reset}`,
  );
  lines.push("");
  lines.push(
    `${ANSI.dim}${truncate("NAME", nameW)}${sep}${truncate("MODEL", modelW)}${sep}${truncate("SESSION", sessW)}${sep}${truncate("WEEKLY", weekW)}${sep}${truncate("COST", costW)}${ANSI.reset}`,
  );
  lines.push(`${ANSI.dim}${"─".repeat(lineWidth)}${ANSI.reset}`);

  for (const entry of agents) {
    const sess = entry.session_percent != null ? `${entry.session_percent}%` : "—";
    const week = entry.weekly_percent != null ? `${entry.weekly_percent}%` : "—";
    const cost = entry.cost != null ? `$${entry.cost}` : "—";
    const model = entry.model || "—";
    const name = entry.name || entry.pane_id;

    const sessColor = entry.session_percent >= 80 ? ANSI.red : entry.session_percent >= 50 ? ANSI.yellow : "";
    const weekColor = entry.weekly_percent >= 80 ? ANSI.red : entry.weekly_percent >= 50 ? ANSI.yellow : "";
    const sessStr = sessColor ? `${sessColor}${truncate(sess, sessW)}${ANSI.reset}` : truncate(sess, sessW);
    const weekStr = weekColor ? `${weekColor}${truncate(week, weekW)}${ANSI.reset}` : truncate(week, weekW);

    lines.push(
      `${truncate(name, nameW)}${sep}${truncate(model, modelW)}${sep}${sessStr}${sep}${weekStr}${sep}${truncate(cost, costW)}`,
    );
  }

  if (agents.length === 0) {
    lines.push(`${ANSI.dim}Waiting for an agent to become idle…${ANSI.reset}`);
  }

  // account-level usage
  const accountPath = path.join(path.dirname(usagePath), "account-usage.json");
  try {
    const acct = JSON.parse(fs.readFileSync(accountPath, "utf8"));
    const platforms = acct.platforms || {};
    if (Object.keys(platforms).length > 0) {
      lines.push("");
      lines.push(`${ANSI.cyan}Account Limits${ANSI.reset}`);
      for (const [name, data] of Object.entries(platforms)) {
        const parts = [];
        if (data.session_percent != null) parts.push(`5h: ${data.session_percent}%`);
        if (data.weekly_percent != null) parts.push(`weekly: ${data.weekly_percent}%`);
        if (data.reset_credits != null) parts.push(`resets: ${data.reset_credits}`);
        if (data.weekly_fable_percent != null) parts.push(`fable: ${data.weekly_fable_percent}%`);
        const updated = data.updated_at ? new Date(data.updated_at).toLocaleTimeString() : "";
        const color = (data.weekly_percent >= 80 || data.session_percent >= 80) ? ANSI.red
          : (data.weekly_percent >= 50 || data.session_percent >= 50) ? ANSI.yellow : ANSI.green;
        lines.push(`${color}  ${name}: ${parts.join(" | ")}${ANSI.reset} ${ANSI.dim}${updated}${ANSI.reset}`);
      }
    }
  } catch (_) {}

  const failures = agents.filter((entry) => entry.error);
  if (error || failures.length > 0) {
    lines.push("");
    lines.push(`${ANSI.yellow}Collection notes${ANSI.reset}`);
    if (error) {
      lines.push(`${ANSI.red}usage.json: ${error}${ANSI.reset}`);
    }
    for (const entry of failures.slice(0, 3)) {
      lines.push(`${ANSI.dim}${entry.name || entry.pane_id}: ${entry.error}${ANSI.reset}`);
    }
  }

  process.stdout.write(`${ANSI.clear}${ANSI.hideCursor}${lines.join("\n")}\n`);
}

function trace(stateDir, msg) {
  try { fs.appendFileSync(path.join(stateDir, "dashboard-trace.log"), `${new Date().toISOString()} ${msg}\n`); } catch (_) {}
}

function main() {
  const stateDir = process.env.HERDR_PLUGIN_STATE_DIR;
  if (!stateDir) {
    throw new Error("HERDR_PLUGIN_STATE_DIR is required");
  }
  trace(stateDir, `start isTTY=${process.stdin.isTTY} pid=${process.pid}`);
  const usagePath = path.join(stateDir, "usage.json");
  let closed = false;

  const cleanup = () => {
    if (closed) return;
    closed = true;
    process.stdout.write(`${ANSI.showCursor}${ANSI.reset}`);
    if (process.stdin.isTTY) process.stdin.setRawMode(false);
  };

  const close = () => { cleanup(); process.exit(0); };

  process.on("exit", cleanup);
  process.on("SIGINT", close);
  process.on("SIGTERM", close);

  trace(stateDir, "before stdin setup");
  try {
    if (process.stdin.isTTY) process.stdin.setRawMode(true);
  } catch (e) { trace(stateDir, `setRawMode failed: ${e.message}`); }
  process.stdin.resume();
  process.stdin.setEncoding("utf8");
  process.stdin.on("data", (input) => {
    trace(stateDir, `stdin data: ${JSON.stringify(input)}`);
    if (input === "\x03" || input === "\x1b" || input.toLowerCase() === "q") close();
  });
  trace(stateDir, "before render");

  render(usagePath);
  trace(stateDir, "after render, starting interval");
  setInterval(() => render(usagePath), 5_000);
}

if (require.main === module) {
  try { main(); } catch (error) {
    process.stderr.write(`usage-monitor dashboard: ${error.message}\n`);
    process.exitCode = 1;
  }
}
