# Command journal / コマンド実行ログ

```sh
cargo run --locked -- --config examples/command_journal/config.rb
```

Enable [shell integration](../../docs/shell-integration.md), run several
commands (including one that fails), and press Alt+J. The read-only panel shows
the latest 50 results across Panes with local completion time, Pane ID, exit
status, monotonic elapsed time and cwd. Type to filter; Escape closes it.
The panel is a snapshot: reopen to see newer entries. Alt+K clears completed
entries. Reload also clears all state. Nothing is written to disk.

Missing start markers show `duration unknown`; absent status shows `exit ?`.
Elapsed time measures receipt of shell events, including script queue delays,
and is not a process profiler. This is a result journal: it does not capture
shell input or screen output. Customize the entry formatter or the 50-entry
limit in `config.rb`. See [shared controls](../README.md).

[シェル連携](../../docs/shell-integration.md)を有効にし、成功・失敗するコマンドを
いくつか実行してAlt+Jを押してください。全ペインを通じた直近50件の完了時刻・ペインID・
終了コード・単調時計による経過時間・cwdを、読み取り専用パネルで表示します。入力で
絞り込み、Escapeで閉じます。表示は開いた時点のスナップショットです。新しい結果は
開き直して確認します。Alt+Kで完了履歴を消去し、再読込でも全状態がリセットされます。
ディスクには保存しません。

開始通知がなければ `duration unknown`、終了コードがなければ `exit ?` になります。
時間はシェルイベントの受信間隔で、Ruby処理待ちも含むためプロファイラーではありません。
シェル入力・画面出力は記録しません。書式や50件の上限は `config.rb` で変更できます。
[共通操作](../README.ja.md)も参照してください。
