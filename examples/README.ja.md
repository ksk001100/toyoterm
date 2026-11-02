# 拡張APIの実用例

[English](README.md)

各シナリオをディレクトリに分け、起動用の `config.rb` とガイドを揃えています。
リポジトリルートから起動します。別の場所からは設定の絶対パスを指定してください。

```sh
cargo run --locked -- --config examples/command_forge/config.rb
```

インストール済みなら `toyoterm --config examples/command_forge/config.rb` で起動できます。

Ruby のレシピではなく Neovim 側の設定例は、
[LazyVim / Snacks の画像連携](neovim/README.ja.md)を参照してください。

## Rubyで端末の振る舞いを作る

| 例 | できること | 操作 |
| --- | --- | --- |
| [Command Forge](command_forge/README.md) | 宣言からRubyメソッド・コマンド・キー・ヘルプを生成し、フックでまとめて装飾 | Alt+1／2／3、Alt+Iで生成結果確認 |
| [プロジェクト別モード](project_modes/README.md) | ペインごとのcwdとメジャーモードにより、同じキーをRust／Node用に切替 | Alt+Enter、Alt+Hで説明 |
| [端末出力レンズ](output_lenses/README.md) | 生成したMethodと独立した提供ライブラリで可視画面を抽出し、検索・コピーへ | Alt+L |
| [ステータスバー](status_bar/README.md) | ワークスペース・cwd・実行状態・非同期Gitブランチ・時計・失敗バッジ | コマンド実行とディレクトリ移動 |
| [動的テーマ](dynamic_theme/README.md) | ローカル時刻に応じた昼夜テーマと手動選択 | Alt+T |
| [ペイン作業台](pane_workbench/README.md) | 同じcwdのシェル・作業用タブ・ワークスペース横断のペイン検索 | Alt+W／N／P |
| [コマンド実行ログ](command_journal/README.md) | 終了状態と所要時間を検索可能なパネルで確認 | Alt+J／K |

## 関連APIをまとめて試す

従来の単体Ruby例はテーマ別にまとめ、共通の入口から試せるように統合しました。
個々のAPIを調べる場合は、各ディレクトリ内の部品を単独でも読み込めます。

| ディレクトリ | 統合した内容 | 起動設定・操作 |
| --- | --- | --- |
| [基本設定](starter/README.md) | 全体の既定設定と最小設定（どちらかを選択） | `starter/config.rb` または `starter/minimal.rb` |
| [コマンド作業台](commands/README.md) | パレット・ヘルプ・型付き引数・ディレクトリ補完 | `commands/config.rb`、Alt+X／H／L／P |
| [ペイン操作](pane_tools/README.md) | 移動モード・一時キー・操作アドバイス・サイズイベント | `pane_tools/config.rb`、Alt+N／S／R／M |
| [状態監視](monitoring/README.md) | ライブパネル・ペインの時計・リマインダー | `monitoring/config.rb`、Alt+M／T／R |
| [画面解析](screen_analysis/README.md) | 協調処理による集計と命令数上限付き要約 | `screen_analysis/config.rb`、Alt+W／I |
| [拡張ライブラリ](extension_libraries/README.md) | 独立した提供・ポリシー・観測フックとスコープによる解除 | `extension_libraries/config.rb`、Alt+P／S／M |
| [性能計測](performance/README.md) | ネイティブとRubyのキー入力から描画までの比較 | `performance/config.rb`、Ctrl+Shift+F11／F12 |

旧 `theme_selector.rb` はテーマ登録も含む `dynamic_theme/config.rb` に統合しました。
旧 `async_bar.rb` は独立した[ネットワークバー版](status_bar/network.rb)へ移動しました。
`--config examples/status_bar/network.rb` で起動し、`ping`／`curl` を使って1.1.1.1と
wttr.inへ通信します。通常のステータスバーとはどちらかを選んでください。
Pythonの描画・画像プローブは検証用ツールとして直下に残しています。

## 操作と自分の設定への取り込み

基本設定・性能計測を除く `config.rb` は[共通キー](shared/keys.rb)を使います。
Ctrl+Shift+Tでタブ作成、Ctrl+Tab／Ctrl+Shift+Tabでタブ移動、Ctrl+Shift+E／Dで右／下分割、
Ctrl+Shift+矢印でペイン移動、Ctrl+Shift+Zでズーム、Ctrl+Shift+Rで再読込します。
コピー／貼付はCtrl+Shift+C／V（macOSはCommand+Shift+C／V）です。
パネル・選択画面は入力を捕捉するのでEscapeで閉じてください。モードが一時的にキーを
上書きする場合があります。ペイン作業台のAlt+Wは毎回シェルを2つ追加します。

cwd・実行状態・実行ログ・自動プロジェクト判定には[シェル連携](../docs/shell-integration.md)
が必要です。gem・アイコンフォント・特定のシェルは不要です。プロジェクト用タスクは
Cargo／npmを必要とし、操作したときだけ実行します。通常バーのGitは省略可能です。
ネットワーク版以外の例は、それ自体では外部サービスへ通信しません。基本設定やパレットの
一部操作はシェルへ文字列を送るため、実行中のシェルで使う前にコードを確認してください。

取り込む場合は部品を `require_relative` で明示的に読み込み、キーを調整し、バーを1つの
`window.bar :bottom` にまとめます。バーの重複定義はできません。関連ファイルと `shared/`
もコピーしてください。再読込成功で登録とメモリ内状態を置き換え、失敗時は以前の設定を保持します。
Rubyオブジェクトや外部への副作用はロールバックしません。詳しくは[Ruby API](../docs/mruby-api.md)へ。
