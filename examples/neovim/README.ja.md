# LazyVim / Snacks の画像表示

[English](README.md)

[snacks.lua](snacks.lua) を Neovim の設定ディレクトリの
`lua/plugins/toyoterm-image.lua` にコピーし、toyoterm 内で Neovim を再起動します。
これは Lazy のプラグイン設定で、toyoterm の Ruby 設定ではありません。
既存の画像オプションを保持しながら Snacks の画像表示を有効にします。

Snacks は現在、Kitty graphics の対応を問い合わせず、Kitty・Ghostty・WezTerm
という端末名で判定します。この例は `snacks.image.terminal.envs()` に toyoterm を登録し、
Unicode placeholders と `remote = true` を指定します。ここでの `remote` は、ローカルでも
PNG データを直接分割送信する方式（`t=d`）を選ぶ指定です。通常のローカル用の
ファイルパス転送（`t=f`）には toyoterm が対応していません。
`TERM` の変更や `force = true`、`SNACKS_KITTY`、`SNACKS_SSH` は不要です。
以前追加した `SNACKS_KITTY` / `SNACKS_SSH` の上書き設定は削除してください。

Snacks 標準の直接転送は、4 KiB ごとに `uv.sleep(1)` を呼び、Neovim の入力処理も止めます。
Windows のタイマー精度の影響で、数秒間操作できなくなる場合があります。
この設定は toyoterm 用の送信処理を置き換え、小さなブロック単位で読み込み・エンコードし、
タイマーのコールバックごとに最大 64 KiB を送って Neovim に制御を返します。
画像は順番に送り、転送中の配置・削除要求も完了まで待たせるため、別バッファの操作で
分割転送が途切れません。送信完了後に配置を有効にします。失敗時は通知してファイルを閉じ、
次の画像の送信を続けます。エンコード後の転送上限は引き続き 32 MiB です。

登録は `TERM_PROGRAM=toyoterm` の場合に限定し、tmux・Zellij 内では適用しません。
toyoterm は tmux passthrough に未対応です。SSH 接続先には通常 `TERM_PROGRAM` が
引き継がれず、この例は SSH の環境変数転送を設定しません。
Snacks の環境登録と、画像の `send`・`ready`・`failed`、端末の `request` メソッドの
ラップを使っています。Snacks 2.31.0 で確認済みです。更新時はこれらの連携箇所を見直して
ください。ImageMagick の変換、端末でのデコード、GPU 転送にも表示時間はかかります。

小さな PNG を `:edit /absolute/path/image.png` で開き、`:checkhealth snacks` を実行します。
画像の項目で `toyoterm` と Unicode placeholders が対応と表示されることを確認してください。
PNG 以外の変換には ImageMagick（`magick`）が必要です。
Kitty・WezTerm・Ghostty の実行ファイルが見つからないと表示されることがありますが、
この連携のためにそれらをインストールする必要はありません。

Markdown 内の画像には LazyVim の `lang.markdown` extra と、health check が示す
Treesitter parser を有効にしてください。スクロール、リサイズ、同じ PNG を別のバッファで
開く操作、バッファの終了を確認します。ネイティブの回帰テストでは直接転送、複数の仮想配置、
配置ごとの削除を検証しますが、GUI の確認は別途必要です。
toyoterm の[画像の制限](../../docs/image-protocols.md)は引き続き適用されます。

インストール済みの Snacks を使った検証は、リポジトリルートで実行します。

```sh
nvim --headless -u NONE -l examples/neovim/check.lua /path/to/snacks.nvim
```

転送中のイベント処理、画像バイト列の転送、配置の準備状態、送信順序、別コマンドの待機、
エラー後の復帰、別の端末では元の送信処理を使うことを確認します。
3 MiB のテストデータの計測は端末への書き込みを捕捉した送信処理の値で、描画全体の計測では
ありません。開発用 Windows 環境では、標準の 1 転送が 13.4 秒、修正後の 2 転送の合計が
約 1.6～1.7 秒でした。
