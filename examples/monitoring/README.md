# Monitoring / 状態監視

```sh
cargo run --locked -- --config examples/monitoring/config.rb
```

Combines `panels.rb` and `timers.rb`. Alt+M opens a live snapshot panel; type
to filter and Escape to dismiss it. The next timer tick cleans up its mode.
Alt+T toggles a Pane-local clock (enabled in the initial Pane at startup);
Escape disables that clock. Alt+R adds a reminder after five seconds. Closing
a Pane or disabling its mode cancels owned timers. Close the panel before using
other bindings because it captures input.

ライブパネルとタイマーの例を統合しました。Alt+Mで端末状態を継続表示し、入力で絞り込み、
Escapeで閉じると次のタイマー周期でモードを片付けます。Alt+Tはペイン固有の時計
（起動時のペインでは自動有効）、Escapeで時計を無効化、Alt+Rは5秒後のリマインダーです。
ペイン終了やモード無効化で所有タイマーを解除します。パネル表示中は入力を捕捉するため、
ほかのキーを使う前に閉じてください。
