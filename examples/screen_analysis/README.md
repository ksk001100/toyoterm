# Screen analysis / 端末画面の解析

```sh
cargo run --locked -- --config examples/screen_analysis/config.rb
```

Compare two approaches to analysing the same kind of terminal snapshot:
Alt+W runs `cooperative.rb`, counting words in small slices with progress and
scheduler statistics; Alt+I runs `bounded.rb`, returning a summary or an
instruction-budget error. Both use only visible screen text, not all scrollback.
Close each panel before trying the other. Cooperative work yields between items;
instruction budgets count Ruby opcodes and cannot interrupt blocking native I/O.
Both components remain individually loadable for focused experiments.

可視画面の単語を解析する2つの方式を統合しました。Alt+Wは小分けの協調処理で進捗と
スケジューラー統計を表示し、Alt+Iは命令数の上限付きで要約または上限到達を表示します。
全スクロールバックは対象にしません。片方のパネルを閉じてから他方を試してください。
協調処理は項目間で譲り、命令予算はRuby命令を数えるため、ネイティブI/Oの停止はできません。
各ファイルだけを読み込んで比較することもできます。
