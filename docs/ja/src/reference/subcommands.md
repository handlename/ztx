# サブコマンド

ztx は 6 つのサブコマンドを提供します。永続的な設定はすべて
[`config.toml`](configuration.md) で管理します。以下のフラグは 1 回の実行にのみ適用されます。

---

## `ztx run`

Agent CLI を ztx の PTY プロキシでラップして実行します。子プロセスが生存している間、ztx のすべての機能（キーバインド、ヒントモード、エクスポート、IPC）が有効になります。

```sh
ztx run [OPTIONS] -- <command> [args...]
```

`--` セパレーターは必須です。それ以降の引数はすべてそのまま子プロセスの argv として渡されます。

| フラグ | 値 | デフォルト | 説明 |
|------|--------|---------|-------------|
| `--adapter` | `auto` \| `claude` \| `antigravity` \| `none` | `auto` | アダプターを選択します。`auto` はコマンド名から自動検出します（`claude` → Claude Code アダプター、`agy` / `antigravity` → Antigravity アダプター、それ以外 → アダプターなし）。 |
| `--title-mode` | `passthrough` \| `managed` \| `prefix` | アダプターが一致する場合は `managed`、それ以外は `passthrough` | 子プロセスからの OSC タイトルシーケンスの処理方法を制御します。`passthrough` はそのまま転送します。`managed` は抑制し、ztx がアダプター駆動のタイトルを送出します。`prefix` は固定文字列でタイトルを書き換えます。 |
| `--title-prefix` | 文字列 | `"<command>: "` | `--title-mode prefix` が有効なときに使用するプレフィックス文字列。 |

### 使用例

```sh
# Claude Code をラップします。アダプターとタイトルモードは自動検出されます。
ztx run -- claude

# 任意のシェルをラップします。すべての機能が PTY キャプチャ品質で動作します。
ztx run --adapter none -- bash

# 未登録の CLI に対してカスタムプレフィックスで managed タイトルを強制します。
ztx run --adapter none --title-mode prefix --title-prefix "work: " -- mycli
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

## `ztx send`

実行中の ztx セッションにファイル参照、行番号、または選択テキストを送信します。メッセージはブラケットペースト (bracketed paste) として注入されるため、Agent CLI は単一ユニットとして受け取ります。Zed タスクから呼び出すことを想定しています（`ztx setup zed` 参照）。

```sh
ztx send [OPTIONS] [message...]
```

| フラグ | 値 | デフォルト | 説明 |
|------|--------|---------|-------------|
| `--from-zed-env` | — | オフ | 明示的なフラグの代わりに、`ZED_RELATIVE_FILE`、`ZED_ROW`、`ZED_SELECTED_TEXT` 環境変数からファイル・行・選択テキストを読み取ります。`$ZED_*` 補間によるシェルインジェクションを避けるため、Zed タスクでの使用が推奨されます。 |
| `--file` | パス文字列 | — | メッセージに含めるファイルパス。 |
| `--line` | 整数 | — | ファイル参照に付加する行番号。 |
| `--text` | 文字列 | — | フェンスコードブロックとして付加する選択テキスト。 |
| `--socket` | パス | プロジェクトソケット | Unix ソケットパスで特定のセッションを指定します。このフラグなしの場合、ztx はプロジェクトディレクトリが `ZED_WORKTREE_ROOT`（またはカレントディレクトリ）と一致するセッションにルーティングします。 |
| `message` | 位置引数、複数ワード可 | — | ファイル・テキストコンテキストの後に追加される自由形式のメッセージテキスト。 |

### 使用例

```sh
# 現在の Zed 選択を注入します（setup zed でインストールされた Zed タスクから呼び出されます）。
ztx send --from-zed-env

# 明示的な参照を注入します。
ztx send --file src/main.rs --line 42 --text "this panics on empty input"

# 特定のセッションを指定します。
ztx send --socket ~/.local/share/ztx/abc123.sock "please review this"
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

---

## `ztx setup zed`

ztx のタスクとキーバインドを生成して Zed の設定にマージします。ztx のインストール後に一度実行してください。ファイルを書き込む前に確認を求め、バックアップを作成します。

```sh
ztx setup zed [OPTIONS]
```

| フラグ | 値 | デフォルト | 説明 |
|------|--------|---------|-------------|
| `--yes` | — | オフ | 確認なしで変更を適用します。 |
| `--preview` | — | オフ | ファイルを書き込まずに変更内容のプレビューを表示します。 |
| `--scope` | `global` \| `project` | `global` | Zed 設定の書き込み先。`global` は `~/.config/zed/` に書き込みます。`project` は `<worktree>/.zed/` に書き込みます（`ZED_WORKTREE_ROOT` を起点とし、未設定の場合はカレントディレクトリ）。Zed はプロジェクトローカルのキーマップをサポートしないため、project スコープではタスクのみ書き込み、キーバインドは手動追加のために表示されます。 |

### 使用例

```sh
# グローバルな Zed 設定へのインタラクティブインストール。
ztx setup zed

# ファイルを書き込まずに変更内容をプレビューします。
ztx setup zed --preview

# 現在のプロジェクトの .zed ディレクトリへの非インタラクティブインストール。
ztx setup zed --scope project --yes
```

> このページの英語版: [Subcommands](../../reference/subcommands.html)
