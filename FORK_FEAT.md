# Fork 機能要件

`zubo334/herdr` fork 固有の機能を管理するドキュメント。
upstream 同期時にはこれらを必ず維持すること。upstream のリファクタで
統合箇所が壊れた場合、新アーキテクチャに合わせて再実装してから
同期完了とする。

## 1. Usage Column（TUI 使用量カラム）

画面右側に表示するプラットフォーム別 API トークン使用量パネル。

- **データ経路:** usage-monitor プラグイン（サーバー側）が
  `$HERDR_PLUGIN_STATE_DIR/account-usage.json` に書き込み →
  サーバーが読み取って `ClientShellSnapshot.plugin_usage` に載せる →
  クライアントが描画。SSH 越しでも動作する。
- **開閉操作:** カラムのタイトルクリックで折り畳み、右端の
  1文字幅トグルストリップをクリックで展開。状態はクライアントの
  chrome preferences に永続化（`usage_column_collapsed`）。
- **表示内容:** プラットフォーム名、5時間/週間/fable 使用量バー
  （緑 < 50% < 黄 < 80% < 赤）、リセットクレジット、
  最終更新タイムスタンプ（UTC → ローカル変換）。

### 関連ファイル

| レイヤー | ファイル |
|----------|----------|
| プロトコル | `src/protocol/wire.rs`（`PluginUsageEntry`, `ClientShellSnapshot.plugin_usage`） |
| サーバー | `src/server/client_shell.rs`（`load_plugin_usage`） |
| クライアント状態 | `src/client/shell/state.rs`（`ClientShellLayout.usage_column/usage_toggle`, `ShellHitMap.usage_column_title/usage_toggle_button`, `ClientShellState.usage_column_collapsed`） |
| クライアントレイアウト | `src/client/shell/config.rs`（`layout()` の `usage_column_collapsed` 引数） |
| クライアント描画 | `src/client/shell/render.rs`（`render_usage_column`, `render_usage_toggle`） |
| クライアント入力 | `src/client/shell/mouse.rs`（トグルクリックハンドラ） |
| 永続化 | `src/client/shell/preferences.rs`（`usage_column_collapsed`） |

## 2. Usage Monitor プラグイン

`plugins/usage-monitor/` に同梱。

- `usage-poller.js` — 起動時プロセス。エージェントの使用量を
  `/usage` および `/status` コマンド経由で 60秒間隔でポーリング。
- `on-status-changed.js` — `pane.agent_status_changed` イベントハンドラ。
  ステータス変化時に即座に使用量ポーリングを実行。
- `dashboard.js` — ポップアップ TUI ダッシュボード（バー表示）。
- `show-usage.js` — プラグインアクションのエントリポイント。

## 3. Fork リリースワークフロー

`.github/workflows/fork-release.yml` — Linux/macOS/Windows バイナリを
ビルドし、Windows 版は ConPTY を同梱パッケージ化。preview チャネル
更新用の `preview.json` を生成。手動ディスパッチでバージョン指定可能。

`.github/workflows/fork-tag-sync.yml` — upstream が新しいタグを
公開した際に fork へ通知。

## 4. アップデートマニフェスト URL オーバーライド

`src/update.rs` と `src/remote/attach.rs` — 環境変数
`HERDR_UPDATE_MANIFEST_URL` および `HERDR_PREVIEW_MANIFEST_URL` で
upstream のハードコードされたマニフェスト URL を上書きし、
fork 独自のアップデートチャネルを配信可能にする。
