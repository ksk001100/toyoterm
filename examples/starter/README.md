# Starting configurations / 設定の出発点

```sh
cargo run --locked -- --config examples/starter/config.rb
cargo run --locked -- --config examples/starter/minimal.rb
```

`config.rb` is the full documented default configuration; `minimal.rb` is a
smaller starting point. They are alternatives: do not require both. Copy the
one you want and edit its bindings. The minimal example's Ctrl+Shift+H and
leader+G send shell text explicitly; inspect before using in a busy shell.
These replace the former `default_config.rb` and `minimal_config.rb` paths.

`config.rb` は全体を確認できる既定設定、`minimal.rb` は小さな出発点です。
両方を同時に読み込まず、片方をコピーして調整してください。最小設定のCtrl+Shift+Hと
leader+Gは明示的にシェルへ文字列を送るため、実行中のシェルでは内容を確認してください。
旧 `default_config.rb`／`minimal_config.rb` を移動したものです。
