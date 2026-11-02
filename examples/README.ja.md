# 拡張APIの実用例

[English](README.md)

Ruby APIを組み合わせて、日常の端末作業に使える設定を用意しました。
リポジトリのルートから起動します。別の場所からは絶対パスを指定してください。

```sh
cargo run --locked -- --config examples/status_bar/config.rb
```

インストール済みなら `toyoterm --config examples/status_bar/config.rb` で起動できます。
`status_bar` を以下のディレクトリ名に置き換えて試せます。

| 例 | できること | 操作 |
| --- | --- | --- |
| [ステータスバー](status_bar/README.md) | ワークスペース・cwd・コマンド状態・非同期Gitブランチ・時計・失敗バッジ | 成功／失敗するコマンドを実行し、ディレクトリを移動 |
| [動的テーマ](dynamic_theme/README.md) | ローカル時刻に応じた昼夜テーマと手動選択 | Alt+Tでテーマ方針を選択 |
| [ペイン作業台](pane_workbench/README.md) | 同じcwdにシェルを追加、作業用タブ、ワークスペース横断のペイン検索 | Alt+W、Alt+N、Alt+P |
| [コマンド実行ログ](command_journal/README.md) | 終了状態と所要時間を検索可能なパネルで確認 | Alt+Jで表示、Alt+Kで消去 |

全例で[共通キー設定](shared/keys.rb)を読み込みます。Ctrl+Shift+Tでタブ作成、
Ctrl+Tab／Ctrl+Shift+Tabでタブ移動、Ctrl+Shift+E／Dで右／下に分割、
Ctrl+Shift+矢印でペイン移動、Ctrl+Shift+Zでズーム、Ctrl+Shift+Rで再読込できます。
コピー／貼付はCtrl+Shift+C／V（macOSはCommand+Shift+C／V）です。
選択画面とパネルはEscapeで閉じます。各例はシェルへのコマンド送信や既存セッションの
終了を行いません。Alt+Wは押すたびにシェルを2つ追加します。

cwd・コマンド実行状態・終了コード・実行ログには、使用するシェルでの
[シェル連携](../docs/shell-integration.md)が必要です。連携なしでも時計・テーマ選択・
ペイン移動は使えます。cwdが未報告なら、新しいシェルは通常の継承先ディレクトリを使います。
外部プログラムはステータスバーの `git` のみで、省略可能です。ネットワークは使いません。
gem・アイコンフォント・Unix固有のコマンド・特定のシェルは不要です。

各例は独立した設定ファイルで、それぞれ下部バーを定義します。自分の設定に取り込む場合は
`require_relative` で明示的に読み込み、キー割当を調整し、ウィジェットを1つの
`window.bar :bottom` にまとめてください。同じ位置のバーは重複定義できません。
コピーする場合は `shared/` と必要な関連ファイルも含めてください。
再読込に成功するとメモリ内の状態と登録がリセットされ、失敗時は以前の設定が保持されます。
スナップショット・コールバック・ロールバックの仕様は[Ruby API](../docs/mruby-api.md)を参照してください。

従来の直下の `.rb` 例も、個別APIを調べるために残しています。
[モード](modes.rb)、[リソーススコープ](resource_scopes.rb)、[補完](completion.rb)、
[協調処理](cooperative_work.rb)などと組み合わせる出発点として使えます。
