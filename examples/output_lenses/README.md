# Output lenses / 端末出力レンズ

```sh
cargo run --locked -- --config examples/output_lenses/config.rb
```

Display [sample.txt](sample.txt) inside the new terminal:
`cat examples/output_lenses/sample.txt` on Unix shells, or
`Get-Content examples/output_lenses/sample.txt` in PowerShell (from the repository
root). Press Alt+L, choose Diagnostics, URLs or TODOs, select a match, then choose
Find in scrollback or Copy to clipboard. Escape cancels at any step.
No external parser, shell integration or network access is required. URLs are
treated as text and never opened automatically.

Ruby captures only the **visible screen** when Alt+L is pressed. A three-step
native selector uses that same capture even if PTY output continues; scrollback
search later looks for the literal captured text. Deduplicated results are capped
at 200. UI labels are escaped/truncated while copying retains the whole match.
Word-based URL extraction recognises whitespace-separated http(s) tokens and
strips simple trailing punctuation; it is not a complete URL parser.

[lenses.rb](lenses.rb) generates a private `scan` method for each block with
`define_method`, retains it as a Method object, and publishes it through a named
hook. [todos.rb](todos.rb) is a separate library contributing to the same chooser.
Add a required library using this pattern; the block receives one line and must
return an Array of Strings (empty for no match):

```ruby
ExampleOutputLenses.register('Build steps') do |line|
  line.start_with?('Compiling ') ? [line.strip] : []
end
```

Use unique lens names. Keep extraction bounded and quick: it runs on the script
thread. This mruby build has no Regexp, so the examples use String operations.
Reload replaces all providers. [Shared controls](../README.md).

起動した端末でサンプルを表示してください。リポジトリルートからUnixシェルでは
`cat examples/output_lenses/sample.txt`、PowerShellでは
`Get-Content examples/output_lenses/sample.txt` を使えます。Alt+LからDiagnostics／URLs／
TODOsを選び、一致項目を選択してスクロールバック検索またはコピーへ進みます。
どの段階でもEscapeで中止できます。外部パーサー・シェル連携・通信は不要で、URLを自動で開きません。

Alt+Lを押した時点の**可視画面だけ**を取得し、出力が進んでもその内容を使い続けます。
検索は選択した文字列をリテラルで探します。重複を除いて最大200件とし、表示ラベルだけを
エスケープ・短縮します。コピーは元の一致文字列です。URL抽出は空白区切りのhttp(s)と簡単な
末尾記号を扱うサンプルで、完全なURL解析ではありません。

`define_method` で生成したscanメソッドをMethodとして保持し、フックで公開します。
`todos.rb` は独立したライブラリが同じUIへレンズを追加する例です。上のパターンで
1行を受け取り文字列の配列（一致なしは空配列）を返すレンズを追加できます。
名前は一意にし、スクリプトスレッドの処理を短く保ってください。正規表現を含まないmrubyで
動くよう文字列操作だけを使っています。再読込で全レンズを置き換えます。[共通操作](../README.ja.md)。
