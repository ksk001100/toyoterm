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

登録は `TERM_PROGRAM=toyoterm` の場合に限定し、tmux・Zellij 内では適用しません。
toyoterm は tmux passthrough に未対応です。SSH 接続先には通常 `TERM_PROGRAM` が
引き継がれず、この例は SSH の環境変数転送を設定しません。
Snacks の環境登録インターフェースを使うため、更新でインターフェースが変わった場合は
この設定を見直してください。

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
