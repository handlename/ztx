# エディタの選択範囲を送る

`ztx send` は現在のプロジェクトで実行中の ztx セッションにファイル参照とオプションの選択テキストを挿入します。セッションはそれをブラケットペースト (bracketed paste) として受け取るため、複数行のコンテンツが行ごとではなくアトミックな挿入として届きます。

挿入されるテキストは次の形式に従います:

```
path/to/file.rs:42 
```python
selected text here
```
```

フェンスの長さは選択範囲内のバックティックの連続より長くなるよう自動的に拡張されるため、フェンスブロックを含む Markdown を選択した場合も正しく処理されます。

## Zed キーバインド (推奨)

`ztx setup zed` を一度実行してタスクとキーバインドをインストールします:

```sh
ztx setup zed          # ~/.config/zed/ にインストール
ztx setup zed --scope project   # ./.zed/ にインストール (タスクのみ)
ztx setup zed --preview         # 書き込まずに変更内容を表示
```

セットアップ後、Zed の任意のエディタバッファでテキストを選択して `cmd-alt-z` を押します。Zed が `ztx send --from-zed-env` を実行し、Zed がすべてのタスクに注入する環境変数から選択の詳細を読み取ります:

| 変数 | 内容 |
|------|------|
| `ZED_RELATIVE_FILE` | アクティブなファイルのパス (ワークツリールートからの相対パス) |
| `ZED_ROW` | カーソルの行番号 |
| `ZED_SELECTED_TEXT` | 現在選択されているテキスト |

コマンドラインで値を渡す代わりに `--from-zed-env` を使用することで、選択テキストをシェルが再実行するのを防ぎます。選択範囲にシェルメタ文字が含まれる場合に重要です。

## Zed 組み込み機能: AddSelectionToThread

Zed 組み込みの `agent::AddSelectionToThread` アクション (デフォルト: `cmd->`) もターミナルスレッドで動作し、ztx のセットアップは不要です。選択範囲をアクティブなスレッドに直接ペーストします。どちらのアプローチも同じセッションに選択範囲を届けるため、ワークフローに合った方を選んでください。

## 明示的なフラグ

Zed の外から `ztx send` を呼び出す場合、または Zed の環境変数なしで特定のファイルと行を指定したい場合は、値を明示的に渡します:

```sh
ztx send --file src/main.rs --line 42 --text "fn main() {}"

# ファイル参照のみ (テキスト本文なし)
ztx send --file src/main.rs --line 10

# フリーフォームメッセージ (ファイル参照なし)
ztx send "please review the last change"
```

## 特定のセッションへのルーティング

デフォルトでは `ztx send` はプロジェクトディレクトリが `ZED_WORKTREE_ROOT` (または現在のディレクトリ) に一致するセッションにルーティングします。別のセッションを明示的に指定するには:

```sh
ztx send --socket /path/to/session.sock --file foo.rs --line 1
```

実行中のすべてのセッションのソケットパスを一覧表示するには `ztx sessions` を使用してください。

複数のセッションが実行されている場合のルーティングの仕組みについては[プロジェクトごとに1セッション](one-session-per-project.md)を参照してください。

> このページの英語版: [Send editor selections](../../guide/send-selections.html)
