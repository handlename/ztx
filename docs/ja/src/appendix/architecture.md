# アーキテクチャ

ztx は **passive-tap PTY プロキシ (PTY proxy)** です。疑似端末を保持し、その中で
エージェント CLI を子プロセスとして実行し、バイト列を双方向にそのまま転送します
—— 唯一の例外は OSC 0/2 のタイトル処理です。サイドチャネル（tap）が子プロセスの
出力をスクロールバック (scrollback) バッファと画面状態フラグへとパースし、各機能は
その状態を読みます。機能がライブのストリームを書き換えることはありません。
ヒントモード (hint mode) のような対話的 UI は、必要になったときにのみ
オルタネートスクリーン (alternate screen) 上に描画され、その間は出力ポンプが
一時停止します。

設計ドキュメントはこのマニュアルではなくリポジトリに置かれています。対象読者が
ユーザーではなくコントリビュータであるためです。

- **[DESIGN.md](https://github.com/handlename/ztx/blob/main/DESIGN.md)** ——
  アプローチ、モジュール構成、そして却下された代替案（たとえば OSC 8 ハイパー
  リンクを注入するためにストリームをインラインで書き換える案）。
- **[REQUIREMENTS.md](https://github.com/handlename/ztx/blob/main/REQUIREMENTS.md)** ——
  このツールが答えている要件。

モジュール単位の詳細は `src/` 内のモジュールドキュメントが担っています。

> このページの英語版: [Architecture](../../appendix/architecture.html)
