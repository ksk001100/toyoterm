# toyoterm

<p align="center">
  <img src="packaging/app-icon.png" alt="toyoterm icon" width="180">
</p>

[English](README.md)

toyotermはRustと組み込みmrubyによる、プログラム可能な実験的ターミナルエミュレータです。
端末処理はネイティブで行い、Rubyを設定、キーバインド、イベント、コマンドに使用します。
自分で使うために作っている個人の実験的プロジェクトです。

> [!IMPORTANT]
> 開発中です。Workspace・Tab・分割Paneは独立したPTYと端末セッションを持ちます。複数OSウィンドウは初回リリース後の対応予定です。

## 機能

- ネイティブPTY、`alacritty_terminal`によるVT解析、`wgpu`と`glyphon`によるGPU描画
- Workspace、Tab、Pane分割・ズームと独立したシェルセッション
- UTF-8・IME入力、Kitty keyboard protocolの段階的拡張、マウストラッキングとSGRマウスレポーティング、長さ制限付きOSC 50実行時フォントファミリー切替、Kitty OSC 66のセルをまたぐ文字と分数倍率表示、入力時に最下部へ戻るスクロールバック、検索、選択、クリップボードへのコピー・貼り付け
- 独立したアイコンタイトル情報を含むOSCタイトル・iTerm2セッション色／ANSI色／リンク色／選択色／カーソル文字色／下線色／ビジュアルベル色／透明背景色／Display P3色／登録済みRubyテーマプリセットを含むパレット／動的色／文字属性特殊色の制御・色スタック、OSC 8リンク、タブ色・サイズ制限付きPaneバッジ・iTerm2の全進捗状態・セッション状態インジケーター、明示許可・サイズ制限付きクリップボードコピー・URL起動・フォーカス要求、標準サウンド・名前付きアイコン・転送アイコンのキャッシュと表示条件・優先度・全デスクトップの応答通知に対応したデスクトップ通知、カーソル花火を含む注意要求、URL自動検出、作業ディレクトリ・リモートホスト・連携バージョン・終了状態付きコマンドマーカーとキャプチャ出力消去・プロンプト移動・iTerm2マーク移動を行うシェル連携（[対応表](docs/osc-support.md)）
- Linux・macOS・Windowsでの7/8ビットSixel・Kitty画像プロトコル（TUIフレームワークが使うUnicode placeholderを含む）・iTerm2 OSC 1337による画像表示と、明示許可・サイズ制限付きiTerm2／Kittyファイルダウンロードおよび固定ルート内に限定したKittyアップロード。子セッションでは外側の端末から継承した古い機能判定用環境変数を除去し、TUI画像ライブラリがtoyotermを正しく判定できるようにします（[対応範囲と実行例](docs/image-protocols.md)）
- mruby 4.0による設定、ネイティブ・Rubyキーバインド、イベント、コマンド、ローカルRubyライブラリ、テーマ、検索可能な選択オーバーレイ
- アトミックな設定リロード、ライブRubyコンソール、ローカルIPC、ウィンドウバーと壁紙（明示許可・固定ルート内に限定したOSC壁紙変更を含む）

## 現在の状態

主な開発環境はLinuxです。CIではLinux・macOS・Windowsのビルド、テスト、パッケージ作成、
GUI起動スモークテストを実行します。実機での対話的な検証は別途必要です。
詳細は[プラットフォーム検証](docs/platform-validation.md)を参照してください。
複数OSウィンドウ、セッション永続化は初回リリースの対象外です。
Rubyの`MuxWindow`は、単一OSウィンドウ内で表示するMux上のウィンドウを表します。

## ビルドと起動

最近の安定版Rust、同梱mrubyをビルドするCコンパイラ、`winit`・`wgpu`に必要な
プラットフォームライブラリが必要です。LinuxではWaylandまたはX11セッションと、
xkbcommon・`pkg-config`などの開発用ライブラリを用意してください。Windowsの描画にはDirectX 12が必要です。

```sh
cargo run --locked
```

最適化ビルドは`cargo build --release --locked`で作成し、Linux・macOSでは
`target/release/toyoterm`を実行します。Windowsではconsoleを表示しないGUI起動に
`target/release/toyoterm-gui.exe`、CLIコマンドに`target/release/toyoterm.exe`を使用します。

リリース成果物は、Linuxでは`install.sh`付きアーカイブ、macOSではDMGまたはappバンドルの
アーカイブ、Windowsでは任意のユーザー単位インストーラ付きportable zipです。
[インストール・更新・削除・チェックサム](docs/packaging.md)を参照してください。

## 設定

組み込みGUIキーバインドはありません。設定ファイルを保存して次のように起動します。

```sh
toyoterm --config /path/to/config.rb
```

```ruby
Toyoterm.configure do |config|
  config.font.family = "monospace"
  config.font.size = 14

  config.keys do
    ctrl_shift("t").new_tab
    ctrl_shift("e").split(:right)
    ctrl_shift("r").reload_config
    primary_shift("c").copy_selection
    primary_shift("v").paste_clipboard
  end
end
```

[minimal_config.rb](examples/minimal_config.rb)または[default_config.rb](examples/default_config.rb)
を出発点にできます。プロンプトのアイコン表示にはNerd Fontが便利です。インストール済みの正確なファミリー名を指定してください。
論理記号のバインドには入力後の文字を使うため、多くのキーボード配列でShiftが必要でも
`key("$")`は`$`に一致します。

設定は`--config`、`TOYOTERM_CONFIG_FILE`、プラットフォーム既定パスの順に選択します。
Linux・macOS・Windowsのパスとエラー時の復旧は[設定の読込](docs/mruby-api.md#loading-configuration)
を参照してください。`toyoterm reload`で再読込し、`toyoterm ruby console`で複数行の定義や
入力間で保持される変数を使ったRubyのライブ変更ができます。

設定とrequireしたRubyライブラリは信頼済みコードとして動作し、ファイル・プロセス・環境変数・クリップボードへ
アクセスできます。サンドボックスではありません。組み込みランタイムはmrubyのため、
CRubyの全標準ライブラリやgemは利用できません。組み込みランタイムでは、mrubyの
標準ライブラリ、数値、メタプログラミング、ファイルシステムAPIを利用できます。
ファイル操作には通常のRuby `File` / `Dir` APIを使用します。socket APIは含まれません。
詳細はAPIリファレンスを参照してください。
外部コマンドの結果は`Toyoterm.spawn`で同期的に取得できるほか、`Toyoterm.async`により
スクリプトスレッドをブロックせずにバックグラウンドで非同期実行できます。戻り値の
`AsyncTask`はウィジェットのクロージャ内に保持できるため、グローバル変数は不要です。
`cwd:`を渡すと指定した作業ディレクトリでコマンドを実行できます。
ステータスバーでは`bar.section(...)`を使うと、`add(...)`と`add_async(...)`ごとに
更新間隔を個別管理しながら、値を区切り文字でまとめて表示できます。
スクリプトAPIでは、`Toyoterm.version`、`Toyoterm.api_version`、機能検出、ネイティブログ、
解除可能な登録、コンテキストに束縛されたアクション、読み取り専用の設定スナップショットを
利用できます。非同期タスクは完了結果をVMレジストリから解放し、コールバック／結果の
キャンセルにも対応します。プレリリースAPIの`find_workspace` / `open_workspace`と
`MuxWindow`についてはAPIリファレンスを参照してください。

ローカルのRubyライブラリは、`config.rb`から
`require "name"`または`require_relative "path"`で明示的に読み込みます。
設定ファイルと同じディレクトリ、およびその`lib/`を探索します。探索先は`$LOAD_PATH`からも
参照でき、requireしたsourceはその場で評価されます。ディレクトリの自動探索は行いません。
required fileでは通常の`Toyoterm.command`、`Toyoterm.on`、`Toyoterm.theme`、
`Toyoterm.configure` APIを使用します。

## 操作とドキュメント

- [利用ガイド](docs/usage.md)：マウス操作、CLI、ログ、トラブルシューティング
- [mruby APIリファレンス](docs/mruby-api.md)：設定、キーバインド、コールバック、Rubyライブラリ、テーマ
- [シェル連携](docs/shell-integration.md)：作業ディレクトリ・リモートホスト・コマンド状態の通知とプロンプト移動
- [ドキュメント一覧](docs/README.md)：利用者・開発者向けガイド

通常のキー入力はシェルへ送信し、Tab・Workspaceのクリックで切り替え、ドラッグでテキストを
選択します。Control+クリック（macOSはCommand+クリック）で許可されたWeb・メールリンクを開きます。
最後のPaneが終了するとアプリも終了します。

## 開発

locked指定の検証コマンドとネイティブスモークテストは[開発ガイド](docs/development.md)、
パッケージ作成は[リリースチェックリスト](docs/releasing.md)を参照してください。
[クレート構成](docs/architecture.md)と[スレッド契約](docs/threading.md)で、ネイティブ側の所有権と
専用スクリプトスレッドを説明しています。静的キーバインドはRubyを呼び出さず、
Rubyコールバックの返すコマンドはメインスレッドで適用します。

## ライセンス

toyotermは[MIT License](LICENSE)で配布します。

リポジトリには公式mruby 4.0.0のamalgamationをMITライセンスのもとで同梱しています。詳細は[サードパーティー通知](THIRD_PARTY_NOTICES.md)と[保存しているmrubyのライセンス](vendor/mruby/LICENSE)を参照してください。Rust依存ライブラリのライセンスはCIで`cargo-deny`を使って検査します。
