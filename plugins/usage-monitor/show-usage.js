"use strict";

const { execSync } = require("node:child_process");

function shellQuote(value) {
  const text = String(value);
  if (process.platform === "win32") {
    return `"${text.replace(/%/g, "%%").replace(/"/g, '""')}"`;
  }
  return `'${text.replace(/'/g, `'\\''`)}'`;
}

const herdr = process.env.HERDR_BIN_PATH;
if (!herdr) {
  process.stderr.write("usage-monitor: HERDR_BIN_PATH is required\n");
  process.exit(1);
}

try {
  const command = [
    herdr,
    "plugin",
    "pane",
    "open",
    "--plugin",
    "usage-monitor",
    "--entrypoint",
    "dashboard",
  ].map(shellQuote).join(" ");
  execSync(command, { env: process.env, stdio: "inherit", windowsHide: true });
} catch (error) {
  process.exit(error.status || 1);
}
