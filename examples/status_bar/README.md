# Status bar / ステータスバー

```sh
cargo run --locked -- --config examples/status_bar/config.rb
```

Show workspace and Pane count on the left, shell command state in the center,
and Git branch and a local clock on the right. A failed command leaves a badge
until the next command starts. Git runs asynchronously at most once every five
seconds; a missing Git or non-repository directory displays `git: unavailable`.
A detached HEAD displays `git: detached`. Other widgets refresh every second.
After switching Panes, the branch may show the previous result until refreshed.

Enable [shell integration](../../docs/shell-integration.md), change to a Git
repository, and run commands that succeed and fail. Without OSC 7, Git uses the
GUI's launch directory, and command state may remain `READY`. Customize the
`section.add` blocks for snapshot data; keep blocking work in `add_async`.
See [shared controls](../README.md).

左にワークスペースとペイン数、中央にコマンド状態、右にGitブランチと時計を表示します。
失敗すると次のコマンド開始までバッジが残ります。Gitは5秒以上の間隔で非同期実行します。
未インストール／リポジトリ外なら `git: unavailable`、detached HEADなら `git: detached`
です。他の項目は1秒間隔です。ペイン移動直後は更新まで前のブランチが残ることがあります。

[シェル連携](../../docs/shell-integration.md)を有効にし、Gitリポジトリへ移動して、
成功・失敗するコマンドを試してください。OSC 7なしではGitはGUI起動時のディレクトリを使い、
コマンド状態は `READY` のままの場合があります。表示は `section.add`、時間がかかる
外部処理は `add_async` を編集します。[共通操作](../README.ja.md)も参照してください。
