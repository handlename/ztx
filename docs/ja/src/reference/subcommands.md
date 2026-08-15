# サブコマンド

ztx は 4 つのサブコマンドを提供します。永続的な設定はすべて
[`config.toml`](configuration.md) で管理します。以下のフラグは 1 回の実行にのみ適用されます。

---

## `ztx run`

Agent CLI を ztx の PTY プロキシでラップして実行します。子プロセスが生存している間、ztx のすべての機能（キーバインド、ヒントモード、エクスポート、IPC）が有効になります。

```sh
ztx run [OPTIONS] -- <command> [args...]
```

`--` セパレーターは必須です。それ以降の引数はすべてそのまま子プロセスの argv として渡されます。

そのため、以下のフラグは **`--` より前** に置く必要があります。`--` の後ろに書くと ztx ではなく子プロセスへの引数になります。たとえば `ztx run -- claude --force` は「`claude --force` を起動する」という意味であり、ztx の `--force` は有効になりません。正しくは `ztx run --force -- claude` です。

| フラグ | 値 | デフォルト | 説明 |
|------|--------|---------|-------------|
| `--adapter` | `auto` \| `claude` \| `antigravity` \| `none` | `auto` | アダプターを選択します。`auto` はコマンド名から自動検出します（`claude` → Claude Code アダプター、`agy` / `antigravity` → Antigravity アダプター、それ以外 → アダプターなし）。 |
| `--title-mode` | `passthrough` \| `managed` \| `prefix` | アダプターが一致する場合は `managed`、それ以外は `passthrough` | 子プロセスからの OSC タイトルシーケンスの処理方法を制御します。`passthrough` はそのまま転送します。`managed` は抑制し、ztx がアダプター駆動のタイトルを送出します。`prefix` は固定文字列でタイトルを書き換えます。 |
| `--title-prefix` | 文字列 | `"<command>: "` | `--title-mode prefix` が有効なときに使用するプレフィックス文字列。 |
| `--force` | — | オフ | 同じプロジェクトでライブセッションが動いていても、確認せずに終了させて新しいセッションを開始します。ターミナルに接続されているかの判定もスキップするため、インタラクティブでない文脈でも動作します。デフォルトは `config.toml` の [`[run] force`](configuration.md) で変更できます。 |
| `--no-force` | — | オフ | `config.toml` で `[run] force = true` にしていても、その実行に限って確認プロンプトを有効に戻します。`--force` と併記した場合は後に書いた方が優先されます。 |

### 使用例

```sh
# Claude Code をラップします。アダプターとタイトルモードは自動検出されます。
ztx run -- claude

# 任意のシェルをラップします。すべての機能が PTY キャプチャ品質で動作します。
ztx run --adapter none -- bash

# 未登録の CLI に対してカスタムプレフィックスで managed タイトルを強制します。
ztx run --adapter none --title-mode prefix --title-prefix "work: " -- mycli

# 既存のライブセッションを確認なしで置き換えます（Zed のタスクなど非対話環境でも動作）。
ztx run --force -- claude
```

---

## `ztx export`

現在のディレクトリの最新セッションのトランスクリプトを Markdown としてエクスポートし、エディターで開きます。実行中のラッパーセッション内から呼び出す場合、`ctrl-] e` も同じ動作をし、さらにライブの PTY キャプチャにアクセスできます。

```sh
ztx export [OPTIONS]
```

| フラグ | 値 | デフォルト | 説明 |
|------|--------|---------|-------------|
| `--adapter` | `auto` \| `claude` \| `antigravity` \| `none` | `auto` | ネイティブトランスクリプトの検索に使用するアダプターを選択します。`auto` と `claude` はどちらも Claude Code のトランスクリプトを試みます。`none` はネイティブトランスクリプトをスキップし、PTY キャプチャのスクロールバック (scrollback) を使用します。 |
| `--stdout` | — | オフ | Markdown をエディターで開く代わりに stdout に出力します。 |

### 使用例

```sh
# 現在のプロジェクトのセッションをエクスポートしてエディターで開きます。
ztx export

# Markdown を別のツールにパイプします。
ztx export --stdout | pbcopy
```

---

## `ztx notify`

実行中のセッションにアクティビティの変化を通知します。Claude Code プラグインフックで使用されます。同じプロジェクトで ztx セッションが実行されていない場合は何も行いません。

```sh
ztx notify [OPTIONS]
```

| フラグ | 値 | デフォルト | 説明 |
|------|--------|---------|-------------|
| `--from-hook` | — | オフ | stdin からフック JSON を読み取り、作業ディレクトリとトランスクリプトパスを導出し、セッションのマネージドタイトルを更新します。Claude Code プラグインフックでの使用が推奨されます。 |
| `--wake` | — | オフ | セッションのマネージドタイトルを即座に更新します。 |
| `--transcript` | パス | — | 次の `export` で使用する正式なトランスクリプトパスを記録します。 |
| `--socket` | パス | プロジェクトソケット | Unix ソケットパスで特定のセッションを指定します。 |

### 使用例

```sh
# Claude Code プラグインフックによって自動的に呼び出されます。
ztx notify --from-hook

# タイトルを手動で更新します（テスト目的など）。
ztx notify --wake

# 正確なトランスクリプトパスを ztx に伝えます。
ztx notify --transcript ~/.claude/projects/-home-user-myproject/session.jsonl
```

---

## `ztx sessions`

実行中のすべての ztx セッションを一覧表示します。セッションごとに PID、ソケットパス、作業ディレクトリを 1 行で出力します。

```sh
ztx sessions
```

オプションはありません。出力例：

```
12345  /tmp/ztx/abc123.sock  /home/user/myproject
67890  /tmp/ztx/def456.sock  /home/user/otherproject
```

> このページの英語版: [Subcommands](../../reference/subcommands.html)
