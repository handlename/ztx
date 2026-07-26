# はじめに

> [!WARNING]
> **ステータス: α品質 —— 利用は自己責任で。**
>
> - **100% バイブコーディング。** コードベースは全て、自然言語による指示から
>   AI エージェントが生成したものです。
> - **インターフェイスは変わります。** 作者が日常的に使いながら開発しているため、
>   フラグ・設定キー・キーバインドは予告や非推奨期間なしに変わりえます。
>   このマニュアルが記述するのは現在の状態であって、安定した契約ではありません。
> - **α品質です。** 粗さが残ります。いかなる保証もなく、特定目的への適合性も
>   保証しません。利用は自己責任でお願いします。
> - **作者自身のためのツールです。** 個人用ツールを、誰かの役に立つかもしれない
>   と考えて公開しているものです。Issue には目を通しますが、返信・修正・
>   Pull Request のマージを約束するものではありません。

> English version: [ztx Manual](../introduction.html)

> [!NOTE]
> 検索について: mdBook の全文検索は英語向けのトークナイザを使うため、日本語の
> 語句では検索できません。コマンド名やフラグ名（`ztx export`、`--adapter` など）
> での検索は機能します。日本語の内容を探す場合はサイドバーの目次をご利用ください。

**ztx** (Zed / Terminal session / eXchange) は、AI エージェント CLI —— Claude Code、
antigravity-cli など —— を Zed のターミナルセッション（agent パネルの
Terminal Threads）に馴染ませる PTY プロキシ (PTY proxy) です。

Zed の Terminal Threads は本物の CLI をネイティブ機能ごと動かせますが、
Zed の ACP ベースの agent セッションが持つ利便性は失われます。
ztx はそれを取り戻します。

| 機能 | 仕組み |
|------|--------|
| **作業内容を追うセッション名** | ztx が OSC タイトルを注入するため、agent パネルのスレッド名がそのセッションの状況を示します（CLI 固有のアダプター (adapter) 経由） |
| **ログからファイルを開く** | `ctrl-] f` で直近の出力に含まれるファイルパスにヒントラベルが重なります。1つ選ぶと `zed <path>:<line>` が開きます。Zed 組み込みのパス検出により cmd+click も機能します |
| **セッションログを Markdown で開く** | `ctrl-] e`（または `ztx export`）でセッションのトランスクリプトを Markdown に変換し、エディタで開きます |

## 次に読むもの

- ztx が初めての方は [インストール](getting-started/installation.md)、続いて
  [最初のセッション](getting-started/first-session.md) から。
- Zed をお使いなら [Zed のセットアップ](getting-started/zed-setup.md) に全 Terminal Thread を自動でラップする方法があります。
- 特定の機能を探しているなら [Guide](guide/session-names.md) を。
- フラグや設定キーを引きたいなら [Reference](reference/subcommands.md) を。
- うまく動かないときは [トラブルシューティング](troubleshooting.md) を。

## 仕組みの概要

ztx は **passive-tap PTY プロキシ**です。疑似端末を保持し、その中でエージェント
CLI を実行し、バイト列を双方向にそのまま中継します —— 唯一の例外は OSC 0/2 の
タイトル処理です。サイドチャネルがそのバイト列を観測して各機能が読む状態を
構築するため、ztx がライブのストリームを書き換えることはありません。
設計の全体は [アーキテクチャ](appendix/architecture.md) を参照してください。
