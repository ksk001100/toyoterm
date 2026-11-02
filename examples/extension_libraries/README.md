# Extension libraries / 拡張ライブラリの合成

```sh
cargo run --locked -- --config examples/extension_libraries/config.rb
```

Combines hook composition with a removable status library. Alt+P asks the
independent directory/provider/policy/observer files in `hooks/` to choose and
validate a directory, opens a shell there and decorates the originating Pane.
Alt+S opens a live scope-owned panel. Close it, then Alt+M unloads/reloads the
entire status library, including its command, active keymap, panels and timers.
Try Alt+S again after unloading: its registration is gone. Loading the library
again restores it. `composition.rb` and `scopes.rb` are also standalone configs.

フックの合成と、取り外し可能な状態表示ライブラリを統合しました。Alt+Pで `hooks/` の
独立した提供・ポリシー・観測ライブラリを通してディレクトリを選び、シェルを開きます。
Alt+Sはスコープ所有のライブパネルです。閉じてAlt+Mを押すと、コマンド・有効キーマップ・
パネル・タイマーを含むライブラリ全体を解除／再登録します。解除後のAlt+Sでは登録がなく、
再登録後は戻ります。`composition.rb` と `scopes.rb` は単独設定としても使えます。
