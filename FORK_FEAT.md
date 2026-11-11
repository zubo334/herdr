# Fork 機能要件

zubo334/herdr fork 固有の機能を管理する。
upstream 同期時はここに示す機能を維持し、統合箇所が変わった場合は新しい構成に合わせて移植する。

## 1. Account Usage カラム

右側の Usage カラムで Codex、Claude Code、Z.AI のアカウント使用量を表示する。
サーバーが Rust プロバイダーで使用量を取得し、既存の account-usage.json を更新する。
描画時はサーバーが保持するスナップショットを読み、ファイル読み込みや API 呼び出しを行わない。

- 旧クライアントには ClientShellSnapshot.plugin_usage で従来の項目を送る。
- 新しいクライアントには任意の JSON メッセージ endpoint.usage.v1 で追加情報を送る。
- 接続先、接続世代、サーバーの起動識別子を照合し、SSH の接続先を切り替えてもデータを混在させない。
- 使用率、小数値、モデル別上限、リセット時刻、プラン、クレジット、取得エラーを表示する。
- 取得に失敗した場合は同じアカウントの直前の値を保持し、取得成功時刻と古い値であることを示す。
- カラムの見出しで折り畳み、タブバーの Usage ボタンで展開する。
- 開閉状態はクライアントの chrome preferences に保存し、初期状態は折り畳む。

既存のバイナリメッセージの構造は変更しない。
使用量の共有状態は JSON API の usage.get でも取得できる。

## 2. Rust による使用量取得

CodexBar のプロバイダー抽象と直接取得処理を crates/usage-core に移植する。
参照元は steipete/CodexBar のコミット 7447bc77c2497323d9c41e40164ce705533493c8 である。
MIT の著作権表示とライセンス全文を同クレートに保持する。

| プロバイダー | 認証情報 | 取得方法 |
|---|---|---|
| Codex | 既存の Codex OAuth 認証ファイル | ChatGPT の使用量 API |
| Claude Code | 既存の OAuth 認証ファイル、または既定の macOS Keychain | Anthropic の使用量 API |
| Z.AI | API キー | Global または BigModel China の上限 API |

認証情報は読み取り専用とし、Codex と Claude の CLI が管理する更新トークンを使用量取得のために交換しない。
期限切れの場合は CLI 側でログインを更新する。
取得とキャッシュ更新はペインの状態変化から独立して実行する。

単一の herdr バイナリから usage fetch、usage get、usage dashboard を使用できる。
plugins/usage-monitor はポップアップの起動だけを担当し、Node.js を必要としない。
裏のチャットペイン、スラッシュコマンドの解析、JSONL のトークン解析、context_usage の報告処理は廃止する。

設定、CLI、JSON の詳細は [Usage Monitor の説明](plugins/usage-monitor/README.md) を参照する。

## 3. Fork リリースワークフロー

`.github/workflows/fork-release.yml` — Linux/macOS/Windows バイナリを
ビルドし、Windows 版は ConPTY を同梱パッケージ化。preview チャネル
更新用の `preview.json` を生成。手動ディスパッチでバージョン指定可能。

`.github/workflows/fork-tag-sync.yml` — 本家からの `sync: upstream` PR を既定ブランチへマージすると、そのマージコミットに fork タグを作り、Fork Release を明示的に起動する。
タグがすでに別のコミットを指している場合は停止し、公開済みのリリースは再作成しない。

fork の push と PR では、Linux/macOS/Windows のビルド・テストと Windows ConPTY パッケージ検証を実行する。
本家用のコミット件名検証、配布ドキュメント検証、issue 操作、Web サイト配信、Nix、Windows ARM64 インストーラー検証は本家リポジトリだけで実行する。
これらのワークフローファイルは upstream 同期用に保持する。

## 4. アップデートマニフェスト URL オーバーライド

`src/update.rs` と `src/remote/attach.rs` — 環境変数
`HERDR_UPDATE_MANIFEST_URL` および `HERDR_PREVIEW_MANIFEST_URL` で
upstream のハードコードされたマニフェスト URL を上書きし、
fork 独自のアップデートチャネルを配信可能にする。
