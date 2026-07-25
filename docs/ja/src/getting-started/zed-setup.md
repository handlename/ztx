# Zed のセットアップ

## 初回セットアップ

任意のディレクトリから一度だけ実行してください:

```sh
ztx setup zed
```

ztx は Zed の設定にタスクとキーバインドの 2 つのエントリを追加してから終了します。書き込み前に各ファイルに対して確認プロンプトが表示されます。

### 追加される内容

**`~/.config/zed/tasks.json`** — `ztx: send selection` という名前のタスク:

```json
{
  "label": "ztx: send selection",
  "command": "ztx",
  "args": ["send", "--from-zed-env"],
  "reveal": "never",
  "hide": "always"
}
```

**`~/.config/zed/keymap.json`** — タスクを起動するバインド:

```json
{
  "context": "Editor",
  "bindings": {
    "cmd-alt-z": ["task::Spawn", { "task_name": "ztx: send selection" }]
  }
}
```

セットアップ後、Zed エディタでテキストを選択して `cmd-alt-z` を押すと、ztx が `file:line` 参照と選択テキストを同じプロジェクトの実行中セッションに注入します。詳細は [エディタ選択の送信](../guide/send-selections.md) を参照してください。

### 安全性

- **確認**: 各ファイルに追加される内容を表示し、書き込み前に `[y/N]` を確認します。確認をスキップするには `--yes` を渡してください。
- **バックアップ**: 既存ファイルを変更する前に、ztx はそのコピーを同じ場所に作成します（例: `tasks.json.ztx.bak`）。
- **コメント**: Zed の JSON ファイルはコメントを含むことがあります。プレーン JSON としてパースできないファイルに対しては、ztx が追加するエントリを表示するだけでファイルを変更しません。
- **冪等性**: エントリがすでに存在する状態で `ztx setup zed` を再実行しても何も起きません。

## フラグ

### `--preview`

何も書き込まずに各ファイルへの追加内容を表示します:

```sh
ztx setup zed --preview
```

### `--yes`

確認なしですべての変更を適用します:

```sh
ztx setup zed --yes
```

### `--scope project`

グローバルの `~/.config/zed/tasks.json` の代わりに、プロジェクトローカルの `<worktree>/.zed/tasks.json` にタスクをインストールします。ワークツリーのルートは `ZED_WORKTREE_ROOT` 環境変数から取得されます。変数が未設定の場合はカレントディレクトリが使われます。

```sh
ztx setup zed --scope project
```

Zed にはプロジェクトローカルのキーマップがないため、`--scope project` ではキーバインドはファイルに書き込まれません。代わりに ztx がキーマップエントリを表示するので、`~/.config/zed/keymap.json` に手動で追加してください（あるいは `--scope project` なしで `ztx setup zed` を一度実行してグローバルに書き込んでください）。

## すべての Terminal Thread を自動でラップする

エージェントパネルで新しい Terminal Thread を開くたびに自動で `ztx run` が実行されるようにするには、Zed の `settings.json` で `terminal_init_command` を設定します:

```json
{
  "agent": {
    "terminal_init_command": "ztx run -- claude"
  }
}
```

`cmd-,` で Zed の設定を開き、`terminal_init_command` を検索して、使いたいエージェント CLI のコマンドを値に設定してください。

## Zed のビルトイン選択ショートカット

Zed にはネイティブアクション `agent::AddSelectionToThread`（`cmd->`）があり、現在のエディタ選択をフォーカス中のエージェントスレッドに送信します。これは Terminal Thread で動作し、ztx のセットアップは不要です。`ztx send` と `cmd-alt-z` とは完全に独立しています。2 つのアプローチは補完的な関係にあります。`cmd->` はアクティブなスレッドに直接送るのに対し、`cmd-alt-z` はどのスレッドにフォーカスがあるかに関係なく、同じプロジェクト内で実行されている ztx セッションにルーティングします。

> このページの英語版: [Zed setup](../../getting-started/zed-setup.html)
