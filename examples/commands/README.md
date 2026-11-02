# Command workbench / コマンド作業台

```sh
cargo run --locked -- --config examples/commands/config.rb
```

Four former standalone examples now share an entry point: `palette.rb`,
`help.rb`, `arguments.rb`, and `completion.rb`. Alt+X browses commands, Alt+O
browses native/Ruby operations, Alt+H inspects help and copies a selected
definition location, Alt+L prompts for a label/zoom, Alt+P completes a directory
and opens a shell there, and Alt+S adds a running/ready badge. Ctrl+X Ctrl+S
invokes project status; Ctrl+X ? and Ctrl+H B show key help.
The project-status command explicitly sends `git status` to the shell; Git is
needed only for that action. The component files can still be loaded separately.
Only one overlay can be open at a time; Escape closes it.

パレット・ヘルプ・型付き引数・ディレクトリ補完の4例を統合しました。
Alt+Xはコマンド、Alt+Oはネイティブ操作も含む一覧、Alt+Hはヘルプと定義位置コピー、
Alt+Lはラベルとズーム指定、Alt+Pはディレクトリ補完とシェル起動、Alt+Sは実行状態バッジです。
Ctrl+X Ctrl+Sでプロジェクト状態を実行し、Ctrl+X ?／Ctrl+H Bでキーのヘルプを開きます。
状態コマンドだけは `git status` をシェルへ送るのでGitが必要です。各部品は単独でも
読み込めます。オーバーレイは1つずつ開き、Escapeで閉じてください。
