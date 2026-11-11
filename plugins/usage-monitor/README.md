# Account usage

Account quotas are collected by the Herdr server through native Rust providers.
This optional plugin only opens the dashboard popup. Neither collection nor the
right-hand Usage column requires Node, this plugin, an agent process, or a chat pane.

The provider implementation is adapted from CodexBar at commit
7447bc77c2497323d9c41e40164ce705533493c8. See
[the source attribution](../../crates/usage-core/NOTICE.md) and
[the MIT notice](../../crates/usage-core/LICENSE-CodexBar).

## Sources

| Provider | Credentials | Usage source |
| --- | --- | --- |
| Codex | `$CODEX_HOME/auth.json` or `~/.codex/auth.json` | ChatGPT OAuth quota API |
| Claude Code | `CLAUDE_SECURESTORAGE_CONFIG_DIR`, then `CLAUDE_CONFIG_DIR`, otherwise `~/.claude/.credentials.json`; default macOS Keychain item when available without prompting | Anthropic OAuth quota API |
| Z.AI | `Z_AI_API_KEY` | Global or BigModel China quota API |

Codex and Claude CLI credentials remain read-only. Expired credentials require
the owning CLI to refresh its login. API keys for ordinary OpenAI/Anthropic API
billing are not substituted for subscription OAuth credentials.

Explicit access-token overrides are `HERDR_CODEX_OAUTH_TOKEN` and
`HERDR_CLAUDE_OAUTH_TOKEN`. The Claude CodexBar token/scopes environment names are
also accepted for compatibility. Never pass tokens as command-line arguments.

For Z.AI China, select `region = "bigmodel-cn"`. Compatible key sources include
`BIGMODEL_API_KEY`, `ZHIPU_API_KEY`, `ZHIPUAI_API_KEY`, `GLM_API_KEY` and CodexBar's
three local key-file paths. `Z_AI_API_HOST` and `Z_AI_QUOTA_URL` are supported;
explicit overrides must use HTTPS and agree with a selected canonical region.
Team usage additionally requires `team = true` plus
`Z_AI_BIGMODEL_ORGANIZATION` and `Z_AI_BIGMODEL_PROJECT`.

## Configuration

Put `usage.toml` in Herdr's configuration directory. A missing file uses these defaults:

```toml
enabled = true
interval_seconds = 60
timeout_seconds = 10

[codex]
enabled = true

[claude]
enabled = true

[zai]
enabled = true
region = "global"
team = false
```

The server schedules the next refresh after the preceding collection finishes;
requests run off its event loop. Refresh intervals are bounded to 30–86400 seconds
and individual HTTP request timeouts to 1–60 seconds. Rate-limit responses apply
their own cooldown.

Each provider can set `credential_file` (Codex/Claude) or `endpoint` for an explicit
HTTPS quota endpoint. The standard provider login remains the default.
`HERDR_USAGE_DISABLED=1` disables background collection for isolated tests or
servers that should not read credentials; explicit `usage fetch` remains available.

## CLI

The collector and dashboard are subcommands of the existing single Herdr binary:

```sh
herdr usage fetch --json
herdr usage fetch --provider codex --write-cache
herdr usage fetch --config /path/to/usage.toml
herdr usage get
herdr usage dashboard
herdr usage dashboard --cache /path/to/account-usage.json --once
herdr usage license
```

`fetch` runs without starting a Herdr server. It returns JSON and exits nonzero
when a requested provider could not refresh. `--write-cache` uses the same OS lock
and atomic cache replacement as server collection; a one-provider fetch preserves
other matching providers. `get` reads the server-owned snapshot through `usage.get`.

The dashboard reads `usage.get` by default, so disabled collection does not keep
showing an old disk cache. An explicit `--cache` selects an offline JSON file.
Press q/Esc to close, arrows or j/k to scroll, and PageUp/PageDown for larger jumps.

## Data and compatibility

The server writes version 2 JSON to the existing
`$HERDR_PLUGIN_STATE_DIR/account-usage.json` location (the usage-monitor plugin
state directory). It keeps the original session/weekly fields for old consumers.
The normalized data adds fractional percentages, named windows, reset dates,
plans, balances, spending caps, reset inventory and independent refresh errors.

Last-good data is retained only for the same credential/account/scope. A failed
refresh keeps its original success timestamp and is marked stale. Missing quota
data is never reported as zero usage. Shared named sessions coordinate collection
through the cache lock.

The legacy shell snapshot and binary codecs are unchanged. Rich display data
travels in the optional JSON control `endpoint.usage.v1` and is bound to the
endpoint's boot and connection generation. Older clients retain the original
column projection.

The old hidden `_monitor` workspace, chat-command scraping, JSONL token scanning,
and pane `context_usage` reporter are retired. The new collector does not create
or drive agent panes. Existing old monitor panes are not automatically closed.
