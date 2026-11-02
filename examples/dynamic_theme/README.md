# Dynamic theme / 動的テーマ

```sh
cargo run --locked -- --config examples/dynamic_theme/config.rb
```

Use a warm light palette from 07:00 to 17:59 local time and a dark palette
otherwise. A global resource scope owns the 60-second timer, so closing the
initial Pane does not stop it. Alt+T chooses day, night or Automatic; Escape
preserves the current policy. Manual selection suspends automatic switching
until Automatic is selected or the config reloads. The bar shows the policy.
No shell integration or external program is needed.

Try choosing the opposite palette, then Automatic. Customize the hour boundary
in `config.rb` and terminal, chrome and ANSI colors in `themes.rb`. Changes
apply to the whole GUI; per-Pane OSC color overrides can still take precedence.
Reload restores automatic mode. See [shared controls](../README.md).

ローカル時刻の7:00〜17:59は暖色のライトテーマ、それ以外はダークテーマになります。
60秒タイマーをグローバルなリソーススコープで所有するため、最初のペインを閉じても続きます。
Alt+Tで昼・夜・Automaticを選択し、Escapeで変更せず戻れます。手動選択後はAutomaticに
戻すか再読込するまで自動切替を停止します。現在の方針はバーで確認できます。
シェル連携も外部プログラムも不要です。

時刻と逆のテーマを選び、Automaticに戻して変化を確認してください。切替時刻は
`config.rb`、端末・UI・ANSI色は `themes.rb` で変更できます。GUI全体に適用しますが、
ペインごとのOSC色指定が優先される場合があります。再読込で自動モードに戻ります。
[共通操作](../README.ja.md)も参照してください。
