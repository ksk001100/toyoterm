# Project modes / プロジェクト別モード

```sh
cargo run --locked -- --config examples/project_modes/config.rb
```

Enable [shell integration](../../docs/shell-integration.md). In one Pane,
`cd` to a Rust project root; in another, to a Node project root. Alt+Enter now
opens `cargo test` or `npm test` in a lower Pane according to the originating
Pane's mode. Merely changing directory never launches a task. The program must
be installed; the project's test command can execute project-defined code.
The task Pane closes on process exit. Alt+H explains the rules and Alt+R rechecks
after creating/deleting a marker without changing directory.

The profiles table generates Pane-local **major modes** and keymaps. The same
Alt+Enter has different meanings simultaneously in different Panes. A named
provider hook chooses Rust for a direct `Cargo.toml`, then Node for a direct
`package.json`. No parent traversal, manifest parsing or remote access occurs.
Outside recognized roots the generated binding disappears and lower layers
apply. The reported cwd must refer to the local filesystem. Without a cwd
report the automatic mode stays off.

Each mode owns its `command_finished` subscription. Moving out of the project
removes that subscription, clears its badge and removes its keymap; moving into
another project replaces the major mode. Reload detects all existing Panes.
The recipe occupies the Pane's major-mode slot and replaces any other major
mode there. Add profiles or a higher-priority `recipe_project_kind` provider to
adapt it; a provider returns a known profile Symbol or nil. Customise launch argv
in the profile rather than typing commands into the live shell.
[Shared controls](../README.md).

[シェル連携](../../docs/shell-integration.md)を有効にし、別々のペインでRust／Nodeの
プロジェクトルートに移動してください。同じAlt+Enterが、各ペインのモードに応じて下側に
`cargo test`／`npm test` を開きます。移動だけでは実行しません。対応ツールのインストールが
必要で、テストはプロジェクト定義のコードを実行します。終了するとタスク用ペインは閉じます。
Alt+Hでルールを確認でき、ファイルを追加／削除した後はAlt+Rで再判定できます。

宣言表からペインローカルなメジャーモードとキーを生成し、フックで種類を判定します。
直下の `Cargo.toml`、`package.json` の順に優先します。親ディレクトリ探索・内容解析・
リモートアクセスは行いません。ローカルのcwdが報告されない場合は自動モードを有効にしません。
対象外へ移動すると割当を外し、下位のキー設定が有効になります。

終了イベントの購読もモードが所有するため、移動やモード切替で古い購読・バッジ・キーを
片付けます。再読込時は既存ペインも再判定します。この例はペインのメジャーモード枠を使い、
他のメジャーモードがあれば置き換えます。プロファイル表、または既知の種類のSymbol／nilを
返す `recipe_project_kind` フックを追加して拡張できます。[共通操作](../README.ja.md)。
