# Zed のセットアップ

ztx を使うために Zed の設定は必要ありません。Terminal Thread で `ztx run -- <cli>` を実行すれば、すべての機能が有効になります。以下の設定は利便性のためのものです。

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

## エディタの選択範囲を送る

Zed のネイティブアクション `agent::AddSelectionToThread`（`cmd->`）が、現在のエディタ選択をアクティブなスレッド（Terminal Thread を含む）に送信します。ztx 側のセットアップは不要で、ztx は意図的に同等の機能を持ちません。バッファで範囲を選択して `cmd->` を押せば、参照がセッションのプロンプトに入ります。

> このページの英語版: [Zed setup](../../getting-started/zed-setup.html)
