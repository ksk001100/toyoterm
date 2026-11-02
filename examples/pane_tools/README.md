# Pane tools / ペイン操作

```sh
cargo run --locked -- --config examples/pane_tools/config.rb
```

Combines `navigation.rb` (Alt+N: global h/j/k/l navigation, Escape to leave),
`transients.rb` (Alt+S: one split direction; Alt+R: repeat h/j/k/l/z operations),
`policy.rb` (Alt+M: Pane-local advice; Escape to disable), and `events.rb`
(grid-size badges on ready/resize). Alt+D requests a right split; with policy
enabled in that Pane it becomes a lower split. Alt+O opens the operation palette.
The policy also suppresses close actions while shell integration reports a
running command. It is an application convention, not an access-control boundary.
Escape/Ctrl+G or an unassigned key cancels transients, consuming that key.
Load the components separately to study a single API.

ナビゲーション・一時キー・操作アドバイス・サイズイベントを統合しました。
Alt+Nで全体のh/j/k/l移動モード、Alt+Sで分割方向選択、Alt+Rでh/j/k/l/zの反復操作、
Alt+Mでペイン固有の操作ポリシーを切り替えます。Alt+Dの右分割はポリシー有効時に
下分割へ変わり、シェル連携が実行中と報告した場合はclose操作を抑止します。
これはアプリ内の慣習であり権限境界ではありません。Alt+Oは操作パレットです。
移動／ポリシーモードはEscapeで終了し、一時キーはEscape／Ctrl+G／未割当キーで
解除します（そのキーは消費します）。個別APIを調べる場合は部品だけを読み込めます。
