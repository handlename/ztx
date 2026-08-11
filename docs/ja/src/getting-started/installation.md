# インストール

## crates.io

ztx は [crates.io](https://crates.io/crates/ztx) で公開されているため、
1つのコマンドで済みます。

```sh
cargo install ztx
```

公開されている crate からコンパイルするため、Rust ツールチェーンが必要です
（1.96 以降。[`rustup`](https://rustup.rs) を使ったインストールを推奨します）。
チェックアウトは不要です。同じコマンドを再実行すればアップグレードできます。

## バイナリ

プラットフォーム向けのビルド済みバイナリを
[リリースページ](https://github.com/handlename/ztx/releases)からダウンロードしてください。
アーカイブを展開し、`ztx` を `PATH` の通った場所（例: `/usr/local/bin`）に配置してください。

## ソースからビルド

最新の安定版 Rust ツールチェーンが必要です
（[`rustup`](https://rustup.rs) を使ったインストールを推奨します）。

```sh
git clone https://github.com/handlename/ztx.git
cd ztx
cargo install --path .
```

## 確認

```sh
ztx --version
```

バージョン番号とビルド時のコミットハッシュが表示されます
（例: `0.1.0 (a1b2c3d)`）。

## オプションの依存関係

macOS デスクトップ通知を使うには、
[`terminal-notifier`](https://github.com/julienXX/terminal-notifier) が `PATH` 上にある必要があります:

```sh
brew install terminal-notifier
```

これは Claude Code プラグインをインストールしている場合にのみ関係します。プラグインの
フックが `ztx notify --from-hook` を呼び出し、Claude が処理を終えたり入力待ちになったりしたときに通知を発火させます。`terminal-notifier` がない場合（または macOS 以外の環境では）、通知は無視され、他の動作には影響しません。`[notify]` 設定については
[設定](../reference/configuration.md) を参照してください。

> このページの英語版: [Installation](../../getting-started/installation.html)
